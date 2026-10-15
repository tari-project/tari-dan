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

use tari_common_types::types::PrivateKey;
use tari_consensus::{
    hotstuff::MAX_PENDING_REQUESTS_PER_NOTIFIER,
    messages::{
        ForeignProposalNotificationMessage,
        ForeignProposalRequestMessage,
        HotstuffMessage,
        MissingTransactionsRequest,
    },
    traits::InboundMessagingError,
};
use tari_consensus_types::{BlockId, Decision};
use tari_ootle_common_types::{Epoch, NodeHeight, SubstateLockType, committee::Committee};
use tari_ootle_transaction::{Transaction, TransactionId};
use tokio::time::sleep;

use crate::support::{
    TEST_NUM_PRESHARDS,
    Test,
    TestAddress,
    TestVnDestination,
    build_transaction_from,
    committee_number_to_shard_group,
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
