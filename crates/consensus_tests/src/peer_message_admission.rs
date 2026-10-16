//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use ootle_byte_type::ToByteType;
use tari_common_types::types::PrivateKey;
use tari_consensus::{
    hotstuff::{
        MAX_MISSING_TRANSACTIONS_RESPONSE_BYTES,
        MAX_PARKED_BLOCKS_PER_PROPOSER_PER_HEIGHT,
        MAX_PENDING_REQUESTS_PER_NOTIFIER,
    },
    messages::{
        ForeignProposalNotificationMessage,
        ForeignProposalRequestMessage,
        HotstuffMessage,
        MissingTransactionsRequest,
        ProposalMessage,
    },
    traits::InboundMessagingError,
};
use tari_consensus_types::{
    BlockId,
    Decision,
    HighPc,
    LockedBlock,
    ProposalCertificate,
    ToSignatureMessage,
    ValidatorSchnorrSignature,
};
use tari_ootle_common_types::{Epoch, NodeHeight, SubstateLockType, committee::Committee};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StorageError,
    consensus_models::{
        Block,
        BlockCommands,
        BlockHeader,
        BookkeepingModel,
        Command,
        LocalOnlyAtom,
        TransactionRecord,
    },
};
use tari_ootle_transaction::{Transaction, TransactionId, args};
use tokio::time::sleep;

use crate::support::{
    TEST_NUM_PRESHARDS,
    Test,
    TestAddress,
    TestVnDestination,
    Validator,
    build_transaction_from,
    committee_number_to_shard_group,
    helpers,
    logging::setup_logger,
};

/// Commits blocks until the transaction pool drains, failing if it takes more than ten blocks.
async fn commit_until_pool_is_empty(test: &mut Test, address: &TestAddress) {
    loop {
        test.on_block_committed().await;
        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(address).get_leaf_block();
        assert!(
            leaf.height < NodeHeight(10),
            "Transaction not committed after {} blocks",
            leaf.height
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_malformed_inbound_message_does_not_interrupt_consensus() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1"]).start().await;
    test.start_epoch(Epoch(1)).await;
    test.wait_for_all_validators_to_start_consensus().await;

    let address = TestAddress::new("1");
    let validator = test.get_validator_mut(&address);
    let mut state = validator.current_state_machine_state.clone();
    state.mark_unchanged();
    validator
        .tx_malformed_message
        .send(InboundMessagingError::InvalidMessage {
            reason: "Message is missing".to_string(),
        })
        .await
        .unwrap();

    let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    commit_until_pool_is_empty(&mut test, &address).await;

    assert!(
        !state.has_changed().unwrap(),
        "consensus state machine left Running (now {})",
        *state.borrow()
    );
    test.stop();
    test.assert_all_validators_committed(tx.id());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn requests_from_an_unregistered_peer_are_not_served() {
    setup_logger();
    let outsider = TestAddress::new("outsider");
    let replies_to_outsider = Arc::new(AtomicUsize::new(0));
    let mut test = Test::builder()
        .add_committee(0, vec!["1"])
        .add_committee(1, vec!["2"])
        .with_message_filter(Box::new({
            let outsider = outsider.clone();
            let replies_to_outsider = replies_to_outsider.clone();
            move |_from, to, _msg| {
                if *to == outsider {
                    replies_to_outsider.fetch_add(1, Ordering::SeqCst);
                    return false;
                }
                true
            }
        }))
        .start()
        .await;
    test.start_epoch(Epoch(1)).await;

    let address = TestAddress::new("1");
    test.wait_for_all_validators_to_start_consensus().await;

    let foreign_shard_group = test.get_validator(&TestAddress::new("2")).shard_group;
    let validator = test.get_validator(&address);
    let block_id = validator.get_leaf_block().block_id;
    for msg in [
        HotstuffMessage::MissingTransactionsRequest(MissingTransactionsRequest {
            request_id: 0,
            epoch: Epoch(1),
            block_id,
            transactions: HashSet::from([TransactionId::from([1u8; 32])]),
        }),
        HotstuffMessage::ForeignProposalRequest(ForeignProposalRequestMessage::ByBlockId {
            block_id,
            for_shard_group: foreign_shard_group,
            epoch: Epoch(1),
        }),
    ] {
        validator
            .tx_inbound_message
            .send((outsider.clone(), msg))
            .await
            .unwrap();
    }

    // Requests are served off the worker loop within milliseconds, so this leaves ample time for a reply.
    sleep(Duration::from_secs(2)).await;
    test.stop();

    assert_eq!(replies_to_outsider.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_missing_transactions_response_stays_within_its_byte_budget() {
    const NUM_TXS: usize = 6;
    const TX_BYTES: usize = 1024 * 1024;

    setup_logger();
    let responder = TestAddress::new("1");
    let requester = TestAddress::new("3");
    let responses = Arc::new(Mutex::new(Vec::new()));
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3"])
        .with_message_filter(Box::new({
            let requester = requester.clone();
            let responses = responses.clone();
            move |_from, to, msg| {
                if let HotstuffMessage::MissingTransactionsResponse(resp) = msg &&
                    *to == requester
                {
                    let bytes = resp.transactions.iter().map(|tx| tx.len()).sum::<usize>();
                    responses.lock().unwrap().push((resp.transactions.len(), bytes));
                    return false;
                }
                true
            }
        }))
        .start()
        .await;
    test.start_epoch(Epoch(1)).await;
    test.wait_for_all_validators_to_start_consensus().await;

    let validator = test.get_validator(&responder);
    let mut transaction_ids = HashSet::new();
    validator
        .state_store
        .with_write_tx(|tx| {
            for seed in 0..NUM_TXS {
                let transaction = Transaction::builder_localnet(Epoch(1))
                    .call_function(Default::default(), "big", args![vec![
                        u8::try_from(seed).unwrap();
                        TX_BYTES
                    ]])
                    .build_and_seal(&PrivateKey::default());
                transaction_ids.insert(transaction.calculate_id());
                TransactionRecord::new(transaction).save(tx)?;
            }
            Ok::<_, StorageError>(())
        })
        .unwrap();

    validator
        .tx_inbound_message
        .send((
            requester,
            HotstuffMessage::MissingTransactionsRequest(MissingTransactionsRequest {
                request_id: 0,
                epoch: Epoch(1),
                block_id: validator.get_leaf_block().block_id,
                transactions: transaction_ids,
            }),
        ))
        .await
        .unwrap();

    sleep(Duration::from_secs(2)).await;
    test.stop();

    let responses = responses.lock().unwrap();
    assert_eq!(responses.len(), 1, "expected one response, got {responses:?}");
    let (num_transactions, bytes) = responses[0];
    assert!(
        num_transactions > 0 && num_transactions < NUM_TXS,
        "the response carried {num_transactions} of {NUM_TXS} transactions"
    );
    assert!(
        bytes <= MAX_MISSING_TRANSACTIONS_RESPONSE_BYTES,
        "the response carried {bytes} bytes, over the {MAX_MISSING_TRANSACTIONS_RESPONSE_BYTES} byte budget"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_notification_from_a_peer_at_its_pending_cap_is_still_followed_up() {
    setup_logger();
    let victim = TestAddress::new("1");
    let requests_sent = Arc::new(AtomicUsize::new(0));
    let mut test = Test::builder()
        .add_committee(0, vec!["1"])
        .add_committee(1, vec!["2", "3"])
        .with_message_filter(Box::new({
            let victim = victim.clone();
            let requests_sent = requests_sent.clone();
            move |from, _to, msg| {
                if *from == victim && matches!(msg, HotstuffMessage::ForeignProposalRequest(_)) {
                    requests_sent.fetch_add(1, Ordering::SeqCst);
                }
                true
            }
        }))
        .start()
        .await;
    test.start_epoch(Epoch(1)).await;
    test.wait_for_all_validators_to_start_consensus().await;

    let shard_group = test.get_validator(&victim).shard_group;
    let tx_inbound = test.get_validator(&victim).tx_inbound_message.clone();
    let notification = |seed: usize| {
        HotstuffMessage::ForeignProposalNotification(ForeignProposalNotificationMessage {
            block_id: BlockId::from([u8::try_from(seed).unwrap(); 32]),
            epoch: Epoch(1),
            shard_groups: vec![shard_group],
        })
    };

    // None of these blocks exist, so every request stays unanswered and the peer reaches its cap
    let num_notifications = MAX_PENDING_REQUESTS_PER_NOTIFIER + 20;
    for seed in 0..num_notifications {
        tx_inbound
            .send((TestAddress::new("2"), notification(seed)))
            .await
            .unwrap();
    }
    sleep(Duration::from_secs(2)).await;
    assert_eq!(requests_sent.load(Ordering::SeqCst), num_notifications);

    // Gossip delivers one copy of each notification, so the next one from the same peer must still be followed up
    tx_inbound
        .send((TestAddress::new("2"), notification(num_notifications)))
        .await
        .unwrap();
    sleep(Duration::from_secs(2)).await;
    assert_eq!(requests_sent.load(Ordering::SeqCst), num_notifications + 1);

    test.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_notification_from_an_epoch_with_a_different_committee_layout_does_not_interrupt_consensus() {
    setup_logger();
    let target = TestAddress::new("1");
    let notifier = TestAddress::new("3");
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .start()
        .await;

    // Epoch 1 had a single committee, so the notifier's shard group then spans every shard
    let epoch_manager = test.get_validator(&target).epoch_manager.clone();
    let previous_committee = {
        let state = epoch_manager.state_lock().await;
        Committee::new(state.committees.values().flat_map(|c| c.iter().cloned()).collect())
    };
    epoch_manager
        .set_epoch_committees(
            Epoch(1),
            HashMap::from([(
                committee_number_to_shard_group(TEST_NUM_PRESHARDS, 0, 1),
                previous_committee,
            )]),
        )
        .await;

    let inputs = test.create_substates_on_vns(TestVnDestination::Committee(0), 1);
    // An output on each committee, so every validator commits a substate
    let outputs = [0, 1]
        .into_iter()
        .flat_map(|committee_no| test.build_outputs_for_committee(committee_no, 1))
        .collect();
    let tx = build_transaction_from(
        Transaction::builder_localnet(Epoch(2))
            .with_inputs(inputs.iter().cloned().map(|i| i.into()))
            .build_and_seal(&PrivateKey::default()),
    );
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx,
        Decision::Commit,
        100,
        inputs
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        outputs,
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx.clone())
        .await;
    test.start_epoch(Epoch(2)).await;

    let shard_groups = [&target, &notifier].map(|address| test.get_validator(address).shard_group);
    let validator = test.get_validator_mut(&target);
    let mut state = validator.current_state_machine_state.clone();
    state.mark_unchanged();
    validator
        .tx_inbound_message
        .send((
            notifier,
            HotstuffMessage::ForeignProposalNotification(ForeignProposalNotificationMessage {
                block_id: BlockId::from([1u8; 32]),
                epoch: Epoch(1),
                shard_groups: shard_groups.to_vec(),
            }),
        ))
        .await
        .unwrap();

    loop {
        test.on_block_committed().await;
        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&target).get_leaf_block();
        assert!(
            leaf.height <= NodeHeight(30),
            "Transaction not committed after {} blocks",
            leaf.height
        );
    }

    assert!(
        !state.has_changed().unwrap(),
        "consensus state machine left Running (now {})",
        *state.borrow()
    );
    test.stop();
    test.assert_all_validators_have_decision(tx.id(), Decision::Commit)
        .await;
    test.assert_all_validators_committed(tx.id());
}

/// A block signed by `proposer` that is otherwise valid on `template`'s chain, extends `justify` and cites a
/// transaction no validator holds, so the receiver can only park it.
fn block_citing_an_unknown_transaction(
    template: &Block,
    justify: &ProposalCertificate,
    proposer: &TestAddress,
    seed: u32,
) -> Block {
    let (secret_key, public_key) = helpers::derive_keypair_from_address(proposer);
    let mut tx_id = [0xab; 32];
    tx_id[..4].copy_from_slice(&seed.to_le_bytes());
    let commands = BlockCommands::init([Command::LocalOnly(LocalOnlyAtom {
        id: TransactionId::new(tx_id),
        decision: Decision::Commit,
        transaction_fee: 0,
        leader_fee: None,
    })])
    .unwrap();
    let header = template.header();
    let mut forged = BlockHeader::create_unsigned(
        header.network(),
        header.protocol_version(),
        justify.calculate_block_id(),
        justify.calculate_id(),
        None,
        justify.height() + NodeHeight(1),
        header.epoch(),
        header.shard_group(),
        public_key.to_byte_type(),
        *header.state_merkle_root(),
        &commands,
        0,
        header.timestamp(),
        *header.epoch_hash(),
        *header.accumulated_data(),
        header.extra_data().clone(),
    )
    .unwrap();
    let signature =
        ValidatorSchnorrSignature::sign(&secret_key, forged.to_signature_message(), &mut rand::rng()).unwrap();
    forged.set_signature(signature.to_byte_type());
    Block::new(forged, justify.clone(), commands, None)
}

fn count_parked(validator: &Validator, blocks: &[Block]) -> usize {
    validator
        .state_store
        .with_read_tx(|tx| {
            blocks
                .iter()
                .map(|block| tx.parked_block_exists(block.id()))
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap()
        .into_iter()
        .filter(|parked| *parked)
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocks_citing_unknown_transactions_do_not_stay_parked() {
    setup_logger();
    let victim = TestAddress::new("1");
    let sender = TestAddress::new("2");
    let mut test = Test::builder().add_committee(0, vec!["1", "2", "3", "4"]).start().await;
    test.start_epoch(Epoch(1)).await;
    test.wait_for_all_validators_to_start_consensus().await;

    test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    commit_until_pool_is_empty(&mut test, &victim).await;

    let (leaf, locked_height, committed) = test
        .get_validator(&victim)
        .state_store
        .with_read_tx(|tx| {
            let leaf = Block::get(tx, test.get_validator(&victim).get_leaf_block().block_id())?;
            let locked_height = LockedBlock::get(tx, Epoch(1))?.height;
            let mut committed = Vec::new();
            let mut block = leaf.clone();
            while !block.height().is_zero() {
                if !block.is_dummy() && block.justify().height() + NodeHeight(1) <= locked_height {
                    committed.push(block.clone());
                }
                block = block.get_parent(tx)?;
            }
            Ok::<_, StorageError>((leaf, locked_height, committed))
        })
        .unwrap();
    assert!(
        !committed.is_empty(),
        "no block at or below locked height {locked_height}"
    );

    let mut seed = 0;
    let mut forge = |template: &Block, justify: &ProposalCertificate| {
        seed += 1;
        block_citing_an_unknown_transaction(template, justify, &sender, seed)
    };

    // A genuine certificate for a height the victim has already locked admits no block worth keeping
    let mut stale = Vec::new();
    for block in &committed {
        for _ in 0..10 {
            stale.push(forge(block, block.justify()));
        }
    }
    // A certificate above the locked height admits blocks the victim may yet need, but only a few per proposer
    let fresh = (0..20).map(|_| forge(&leaf, leaf.justify())).collect::<Vec<_>>();
    // A committee member may relay a block, but only a committee member's own proposal is kept
    let relayed = (0..5)
        .map(|seed| block_citing_an_unknown_transaction(&leaf, leaf.justify(), &TestAddress::new("outsider"), seed))
        .collect::<Vec<_>>();

    let sentinel = block_citing_an_unknown_transaction(&leaf, leaf.justify(), &TestAddress::new("3"), u32::MAX);
    let sentinel = send_blocks_until_sentinel_is_parked(
        &test,
        &victim,
        &sender,
        stale.iter().chain(&relayed).chain(&fresh),
        sentinel,
        HotstuffMessage::new_catch_up_sync_response,
    )
    .await;

    assert_eq!(count_parked(test.get_validator(&victim), &stale), 0);
    assert_eq!(count_parked(test.get_validator(&victim), &relayed), 0);
    let parked_fresh = count_parked(test.get_validator(&victim), &fresh);
    assert_eq!(
        parked_fresh, MAX_PARKED_BLOCKS_PER_PROPOSER_PER_HEIGHT,
        "one proposer's parked blocks at one height"
    );

    let tx = commit_until_locked_past(&mut test, &victim, leaf.height()).await;
    assert_eq!(count_parked(test.get_validator(&victim), &fresh), 0);
    assert_eq!(count_parked(test.get_validator(&victim), &sentinel), 0);

    test.stop();
    test.assert_all_validators_committed(&tx);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proposals_citing_unknown_transactions_do_not_stay_parked() {
    setup_logger();
    let victim = TestAddress::new("1");
    let sender = TestAddress::new("2");
    let mut test = Test::builder().add_committee(0, vec!["1", "2", "3", "4"]).start().await;
    test.start_epoch(Epoch(1)).await;
    test.wait_for_all_validators_to_start_consensus().await;

    test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    commit_until_pool_is_empty(&mut test, &victim).await;

    // Blocks for the view the victim is in pass the proposal path's view and height checks
    let (high_qc, template) = test
        .get_validator(&victim)
        .state_store
        .with_read_tx(|tx| {
            let high_pc = HighPc::get(tx, Epoch(1))?;
            let high_qc = tx.proposal_certificates_get(Epoch(1), high_pc.id())?;
            let template = Block::get(tx, &high_qc.calculate_block_id())?;
            Ok::<_, StorageError>((high_qc, template))
        })
        .unwrap();

    // Distinct proposals from one member for one view, whether or not that member leads it
    let flood = (0..20)
        .map(|seed| block_citing_an_unknown_transaction(&template, &high_qc, &sender, seed))
        .collect::<Vec<_>>();
    let sentinel = block_citing_an_unknown_transaction(&template, &high_qc, &TestAddress::new("3"), u32::MAX);
    let sentinel =
        send_blocks_until_sentinel_is_parked(&test, &victim, &sender, &flood, sentinel, HotstuffMessage::new_proposal)
            .await;

    assert_eq!(
        count_parked(test.get_validator(&victim), &flood),
        MAX_PARKED_BLOCKS_PER_PROPOSER_PER_HEIGHT,
        "one proposer's parked blocks at one height"
    );

    let tx = commit_until_locked_past(&mut test, &victim, high_qc.height() + NodeHeight(1)).await;
    assert_eq!(count_parked(test.get_validator(&victim), &flood), 0);
    assert_eq!(count_parked(test.get_validator(&victim), &sentinel), 0);

    test.stop();
    test.assert_all_validators_committed(&tx);
}

/// Sends `blocks` and then `sentinel` to `victim` as `to_message` builds them, and waits until `sentinel` is parked.
/// `sentinel` must be another proposer's block, which is parked whatever the sender's count, so that once it is parked
/// every block sent before it has been processed.
async fn send_blocks_until_sentinel_is_parked<'a>(
    test: &Test,
    victim: &TestAddress,
    sender: &TestAddress,
    blocks: impl IntoIterator<Item = &'a Block>,
    sentinel: Block,
    to_message: fn(ProposalMessage) -> HotstuffMessage,
) -> [Block; 1] {
    let tx_inbound = test.get_validator(victim).tx_inbound_message.clone();
    let sentinel = [sentinel];
    for block in blocks.into_iter().cloned().chain(sentinel.iter().cloned()) {
        tx_inbound
            .send((
                sender.clone(),
                to_message(ProposalMessage {
                    block,
                    foreign_proposals: vec![],
                }),
            ))
            .await
            .unwrap();
    }

    for _ in 0..200 {
        if count_parked(test.get_validator(victim), &sentinel) == 1 {
            return sentinel;
        }
        sleep(Duration::from_millis(100)).await;
    }
    panic!("the victim did not process the blocks");
}

/// Commits a transaction and waits until `victim` has locked a block above `height`. Once the chain locks past them,
/// no parked block at or below `height` can ever be voted on.
async fn commit_until_locked_past(test: &mut Test, victim: &TestAddress, height: NodeHeight) -> TransactionId {
    let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    loop {
        test.on_block_committed().await;
        let locked = test
            .get_validator(victim)
            .state_store
            .with_read_tx(|tx| LockedBlock::get(tx, Epoch(1)))
            .unwrap();
        if test.is_transaction_pool_empty() && locked.height > height {
            return *tx.id();
        }
        assert!(
            locked.height < height + NodeHeight(20),
            "chain did not lock past the parked blocks"
        );
    }
}

/// Commits a transaction with inputs on committee 0 and outputs on committees 1 and 2.
async fn commit_cross_shard_transaction(test: &mut Test) {
    let inputs = test.create_substates_on_vns(TestVnDestination::Committee(0), 2);
    let outputs_1 = test.build_outputs_for_committee(1, 1);
    let outputs_2 = test.build_outputs_for_committee(2, 1);
    let tx = build_transaction_from(
        Transaction::builder_localnet(Epoch(1))
            .with_inputs(inputs.iter().cloned().map(|i| i.into()))
            .build_and_seal(&PrivateKey::default()),
    );
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx,
        Decision::Commit,
        5,
        inputs
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        outputs_1.into_iter().chain(outputs_2).collect(),
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx.clone())
        .await;
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;
        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        assert!(
            leaf.height <= NodeHeight(30),
            "Transaction not committed after {} blocks",
            leaf.height
        );
    }
    test.assert_all_validators_committed(tx.id());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn foreign_proposals_are_only_served_to_members_of_the_requested_shard_group() {
    setup_logger();
    let outsider = TestAddress::new("outsider");
    let foreign_member = TestAddress::new("3");
    let notified_blocks = Arc::new(Mutex::new(Vec::new()));
    let armed = Arc::new(AtomicBool::new(false));
    let proposals_to_outsider = Arc::new(AtomicUsize::new(0));
    let proposals_to_foreign_member = Arc::new(AtomicUsize::new(0));
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .add_committee(2, vec!["5", "6"])
        .with_message_filter(Box::new({
            let outsider = outsider.clone();
            let foreign_member = foreign_member.clone();
            let notified_blocks = notified_blocks.clone();
            let armed = armed.clone();
            let proposals_to_outsider = proposals_to_outsider.clone();
            let proposals_to_foreign_member = proposals_to_foreign_member.clone();
            move |from, to, msg| {
                match msg {
                    HotstuffMessage::ForeignProposalNotification(n) if *from == TestAddress::new("1") => {
                        notified_blocks.lock().unwrap().push(n.block_id);
                    },
                    HotstuffMessage::ForeignProposal(_) if armed.load(Ordering::SeqCst) => {
                        if *to == outsider {
                            proposals_to_outsider.fetch_add(1, Ordering::SeqCst);
                        }
                        if *to == foreign_member {
                            proposals_to_foreign_member.fetch_add(1, Ordering::SeqCst);
                        }
                    },
                    _ => {},
                }
                *to != outsider
            }
        }))
        .start()
        .await;

    commit_cross_shard_transaction(&mut test).await;

    let block_id = *notified_blocks
        .lock()
        .unwrap()
        .first()
        .expect("committee 0 sent no foreign proposal notification");
    let shard_group_of = |address: &str| test.get_validator(&TestAddress::new(address)).shard_group;
    let committee_1 = shard_group_of("3");
    let committee_2 = shard_group_of("5");
    let request = |for_shard_group| {
        HotstuffMessage::ForeignProposalRequest(ForeignProposalRequestMessage::ByBlockId {
            block_id,
            for_shard_group,
            epoch: Epoch(1),
        })
    };
    let tx_inbound = test.get_validator(&TestAddress::new("1")).tx_inbound_message.clone();
    armed.store(true, Ordering::SeqCst);

    tx_inbound.send((outsider.clone(), request(committee_1))).await.unwrap();
    tx_inbound
        .send((foreign_member.clone(), request(committee_2)))
        .await
        .unwrap();
    sleep(Duration::from_secs(2)).await;
    assert_eq!(proposals_to_outsider.load(Ordering::SeqCst), 0);
    assert_eq!(proposals_to_foreign_member.load(Ordering::SeqCst), 0);

    // The same block is served to a member of the shard group it asks for, so the requests above were refused
    // by the gate and not for want of a proposal to serve.
    tx_inbound
        .send((foreign_member.clone(), request(committee_1)))
        .await
        .unwrap();
    sleep(Duration::from_secs(2)).await;
    assert_eq!(proposals_to_foreign_member.load(Ordering::SeqCst), 1);

    test.stop();
}
