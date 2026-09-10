//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! # Consensus tests
//!
//! How to debug the database:
//!
//! Use `Test::builder().with_rocks_path("/tmp/test{}")...` to create a database for each validator
//! where {} is replaced with the node address.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use log::info;
use ootle_byte_type::ToByteType;
use tari_common_types::types::PrivateKey;
use tari_consensus::{hotstuff::HotStuffError, messages::HotstuffMessage};
use tari_consensus_types::Decision;
use tari_crypto::tari_utilities::ByteArray;
use tari_engine_types::{
    ValidatorFeeWithdrawal,
    commit_result::AbortReason,
    hashing::hash_template_code,
    published_template::PublishedTemplateAddress,
    substate::SubstateId,
};
use tari_ootle_common_types::{
    Epoch,
    NodeHeight,
    SubstateLockType,
    SubstateRequirement,
    ToSubstateAddress,
    VersionedSubstateId,
    crypto::{create_key_pair, create_key_pair_from_seed},
    derive_fee_pool_address,
    optional::Optional,
    shard::Shard,
};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{Block, Command, SubstateRecord, TransactionRecord},
};
use tari_ootle_transaction::{Transaction, args};

use crate::support::{
    ExecuteSpec,
    Test,
    TestAddress,
    TestVnDestination,
    build_transaction_from,
    load_binary_fixture,
    logging::setup_logger,
};

// Although these tests will pass with a single thread, we enable multi-threaded mode so that any unhandled race
// conditions can be picked up, plus tests run a little quicker.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_transaction() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1"]).start().await;
    // First get transaction in the mempool
    let (tx1, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(10) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    test.stop();
    test.assert_all_validators_committed(tx1.id());

    // Assert all LocalOnly
    test.get_validator(&TestAddress::new("1"))
        .state_store
        .with_read_tx(|tx| {
            let leaf = tx.leaf_block_get(Epoch(1))?;
            let mut block = Block::get(tx, &leaf.block_id)?;
            loop {
                block = block.get_parent(tx)?;
                if block.id().is_zero() {
                    break;
                }

                for cmd in block.commands() {
                    assert!(matches!(cmd, Command::LocalOnly(_)));
                }
            }
            Ok::<_, HotStuffError>(())
        })
        .unwrap();
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;

    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_transaction_multi_vn() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1", "2"]).start().await;
    let (tx1, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(10) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    test.stop();
    test.assert_all_validators_committed(tx1.id());

    // Assert all LocalOnly
    test.get_validator(&TestAddress::new("1"))
        .state_store
        .with_read_tx(|tx| {
            let leaf = tx.leaf_block_get(Epoch(1))?;
            let mut block = Block::get(tx, &leaf.block_id)?;
            loop {
                block = block.get_parent(tx)?;
                if block.id().is_zero() {
                    break;
                }

                for cmd in block.commands() {
                    assert!(matches!(cmd, Command::LocalOnly(_)));
                }
            }
            Ok::<_, HotStuffError>(())
        })
        .unwrap();
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;

    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_transaction_abort() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1"]).start().await;
    // First get transaction in the mempool
    let (tx1, _, _) = test
        .send_transaction_to_all(Decision::Abort(AbortReason::ExecutionFailure), 1, 1, 1)
        .await;
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(10) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    test.stop();
    test.assert_all_validators_have_decision(tx1.id(), Decision::Abort(AbortReason::ExecutionFailure))
        .await;

    test.assert_clean_shutdown().await;
}

/// An abort commits no state, so consensus must accept the identical transaction again and give it a
/// full lifecycle: the resubmission may commit if conditions have changed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_transaction_abort_then_resubmit_is_sequenced_again() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1"]).start().await;
    let (tx1, inputs, new_outputs) = test
        .send_transaction_to_all(Decision::Abort(AbortReason::ExecutionFailure), 1, 1, 1)
        .await;
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(10) {
            panic!("Transaction not finalized after {} blocks", leaf.height);
        }
    }

    test.assert_all_validators_have_decision(tx1.id(), Decision::Abort(AbortReason::ExecutionFailure))
        .await;
    test.assert_all_validators_did_not_commit(tx1.id());

    // Resubmit the identical transaction; this time execution commits.
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx1,
        Decision::Commit,
        1,
        inputs
            .iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        new_outputs,
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx1.clone())
        .await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(20) {
            panic!("Resubmitted transaction not finalized after {} blocks", leaf.height);
        }
    }

    test.stop();
    test.assert_all_validators_committed(tx1.id());
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;

    test.assert_clean_shutdown().await;
}

/// A commit consumes the transaction id permanently: resubmitting a committed transaction must be
/// ignored. The resubmission's execution spec is set to abort so that, if consensus wrongly gave the
/// id a new lifecycle, the finalized decision would visibly change and fail the final assertion.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resubmitted_committed_transaction_is_ignored() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1"]).start().await;
    let (tx1, inputs, _) = test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(10) {
            panic!("Transaction not finalized after {} blocks", leaf.height);
        }
    }

    test.assert_all_validators_committed(tx1.id());

    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx1,
        Decision::Abort(AbortReason::ExecutionFailure),
        1,
        inputs
            .iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        vec![],
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx1.clone())
        .await;

    // A subsequent transaction on the same channel guarantees the resubmission has been processed by
    // the time the pool drains.
    let (tx2, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(20) {
            panic!("Not all transactions finalized after {} blocks", leaf.height);
        }
    }

    test.stop();
    test.assert_all_validators_committed(tx2.id());
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;

    test.assert_clean_shutdown().await;
}

/// The receipt substate is the durable proof of commit: even when a node's finalized records are
/// gone (pruned by GC, or a freshly synced node that never had them), a resubmitted committed
/// transaction must still be refused because its receipt exists in synced state.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resubmitted_committed_transaction_is_ignored_after_records_pruned() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1"]).start().await;
    let (tx1, inputs, _) = test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(10) {
            panic!("Transaction not finalized after {} blocks", leaf.height);
        }
    }

    test.assert_all_validators_committed(tx1.id());

    // Forget the finalized bookkeeping, as record GC would.
    test.get_validator(&TestAddress::new("1"))
        .state_store
        .with_write_tx(|tx| tx.transactions_finalized_remove(tx1.id()))
        .unwrap();

    // Tripwire: if consensus wrongly gave the id a new lifecycle, it would finalize as an abort and
    // the final assertion would see a finalized record reappear.
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx1,
        Decision::Abort(AbortReason::ExecutionFailure),
        1,
        inputs
            .iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        vec![],
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx1.clone())
        .await;

    // A subsequent transaction on the same channel guarantees the resubmission has been processed by
    // the time the pool drains.
    let (tx2, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(20) {
            panic!("Not all transactions finalized after {} blocks", leaf.height);
        }
    }

    test.stop();
    test.assert_all_validators_committed(tx2.id());
    let finalized = test
        .get_validator(&TestAddress::new("1"))
        .state_store
        .with_read_tx(|tx| TransactionRecord::is_record_finalized(tx, tx1.id()))
        .unwrap();
    assert!(
        !finalized,
        "resubmitted committed transaction was re-sequenced despite its receipt existing in state"
    );

    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn propose_blocks_with_queued_up_transactions_until_all_committed() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2", "3", "4", "5"])
        .start()
        .await;
    // First get all transactions in the mempool
    for _ in 0..10 {
        test.send_transaction_to_all(Decision::Commit, 1, 5, 1).await;
    }
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height > NodeHeight(20) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    test.stop();
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn propose_blocks_with_new_transactions_until_all_committed() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1", "2"]).start().await;
    let mut remaining_txs = 10;
    test.start_epoch(Epoch(1)).await;
    loop {
        if remaining_txs > 0 {
            test.send_transaction_to_all(Decision::Commit, 1, 5, 1).await;
        }
        remaining_txs -= 1;
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height > NodeHeight(20) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    test.stop();
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn node_requests_missing_transaction_from_local_leader() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1", "2"]).start().await;
    // First get all transactions in the mempool of node "2". We send to "2" because it is the leader for the next
    // block. We could send to "1" but the test would have to wait for the block time to be hit and block 1 to be
    // proposed before node "1" can propose block 2 with all the transactions.
    let mut tx_ids = Vec::with_capacity(10);
    for _ in 0..10 {
        let (transaction, inputs) = test.build_transaction(5);
        tx_ids.push(*transaction.id());
        // All VNs will decide the same thing
        test.create_execution_at_destination_for_transaction(
            TestVnDestination::All,
            &transaction,
            Decision::Commit,
            5,
            inputs
                .into_iter()
                .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
                .collect(),
            vec![],
        );

        test.send_transaction_to_destination(TestVnDestination::Address(TestAddress::new("2")), transaction)
            .await;
    }
    test.start_epoch(Epoch(1)).await;
    loop {
        let (_, _, _, committed_height) = test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        if committed_height >= NodeHeight(10) {
            test.dump_pool_info();
            test.dump_blocks(&TestAddress::new("1"));
            panic!("Not all transaction committed after {} blocks", committed_height);
        }
    }

    test.stop();
    test.assert_all_validators_committed(&tx_ids[0]);
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multi_shard_single_transaction() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1"])
        .add_committee(1, vec!["2"])
        .start()
        .await;

    let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 105, 2, 2).await;

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("2")).get_leaf_block();
        if leaf1.height > NodeHeight(30) || leaf2.height > NodeHeight(30) {
            panic!(
                "Not all transaction committed after {}/{} blocks",
                leaf1.height, leaf2.height,
            );
        }
    }

    test.stop();
    test.assert_all_validators_have_decision(tx.id(), Decision::Commit)
        .await;
    test.assert_all_validators_committed(tx.id());

    // Each involved shard group accumulates only its portion of the transaction's exhaust burn, so the network-wide
    // sum counts the burn exactly once. Fee 105 with the fixture's 5% burn rate and 2 involved shard groups pays
    // each leader 52 and burns 6: the 5-unit executor-collected burn plus the 1-unit remainder from dividing
    // the fee between the leaders (see calculate_leader_fee); the burn splits 3/3 across the shard groups.
    let mut network_total_burn = 0u128;
    for (vn, expected_burn) in [("1", 3u128), ("2", 3)] {
        let accumulated_burn = test
            .get_validator(&TestAddress::new(vn))
            .state_store
            .with_read_tx(|tx| {
                let leaf = tx.leaf_block_get(Epoch(1))?;
                let block = Block::get(tx, &leaf.block_id)?;
                Ok::<_, HotStuffError>(block.header().total_accumulated_exhaust_burn())
            })
            .unwrap();
        assert_eq!(
            accumulated_burn, expected_burn,
            "unexpected accumulated burn for validator {vn}"
        );
        network_total_burn += accumulated_burn;
    }
    assert_eq!(network_total_burn, 6);

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multi_validator_propose_blocks_with_new_transactions_until_all_committed() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2", "3", "4", "5"])
        .start()
        .await;
    let mut remaining_txs = 10u32;

    test.start_epoch(Epoch(1)).await;
    loop {
        if remaining_txs > 0 {
            test.send_transaction_to_all(Decision::Commit, 1, 5, 1).await;
        }
        test.on_block_committed().await;
        remaining_txs = remaining_txs.saturating_sub(1);

        if remaining_txs == 0 && test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height > NodeHeight(20) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    test.stop();

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multi_shard_propose_blocks_with_new_transactions_until_all_committed() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2", "3"])
        .add_committee(1, vec!["4", "5", "6"])
        .add_committee(2, vec!["7", "8", "9"])
        .start()
        .await;

    let mut tx_ids = Vec::new();
    for _ in 0..20 {
        let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 100, 2, 1).await;
        tx_ids.push(*tx.id());
    }

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("4")).get_leaf_block();
        let leaf3 = test.get_validator(&TestAddress::new("7")).get_leaf_block();
        if leaf1.height > NodeHeight(30) || leaf2.height > NodeHeight(30) || leaf3.height > NodeHeight(30) {
            panic!(
                "Not all transaction committed after {}/{}/{} blocks",
                leaf1.height, leaf2.height, leaf3.height
            );
        }
    }

    test.stop();
    test.assert_all_validators_committed(&tx_ids[0]);

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn foreign_shard_group_decides_to_abort() {
    setup_logger();
    let mut test = Test::builder()
        // TODO: investigate, test can take longer than expected
        .with_test_timeout(Duration::from_secs(60))
        .add_committee(0, vec!["1", "2", "3"])
        .add_committee(1, vec!["4", "5", "6"])
        .start()
        .await;

    let (tx1, inputs) = test.build_transaction(5);
    test.send_transaction_to_destination(TestVnDestination::Committee(0), tx1.clone())
        .await;

    // Change the decision on committee 1 to Abort when executing. This test is not technically valid, as all
    // non-byzantine nodes MUST have the same decision given the same pledges. However, this does test that is it not
    // possible for others to COMMIT without all committees agreeing to COMMIT.
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::Committee(0),
        &tx1,
        Decision::Commit,
        5,
        inputs
            .iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        vec![],
    )
    .create_execution_at_destination_for_transaction(
        TestVnDestination::Committee(1),
        &tx1,
        Decision::Abort(AbortReason::ExecutionFailure),
        5,
        inputs
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        vec![],
    );

    test.send_transaction_to_destination(TestVnDestination::Committee(1), tx1.clone())
        .await;

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("2")).get_leaf_block();
        if leaf1.height > NodeHeight(50) || leaf2.height > NodeHeight(50) {
            panic!(
                "Not all transaction committed after {}/{} blocks",
                leaf1.height, leaf2.height,
            );
        }
    }

    test.stop();
    test.assert_all_validators_have_decision(tx1.id(), Decision::Abort(AbortReason::ExecutionFailure))
        .await;

    test.assert_clean_shutdown().await;
    log::info!("total messages sent: {}", test.network().total_messages_sent());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_local_inputs_foreign_outputs() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        // Two output-only committees
        .add_committee(1, vec!["3", "4"])
        .add_committee(2, vec!["5", "6"])
        .start()
        .await;

    let inputs = test.create_substates_on_vns(TestVnDestination::Committee(0), 2);
    let outputs_1 = test.build_outputs_for_committee(1, 1);
    let outputs_2 = test.build_outputs_for_committee(2, 1);

    let tx1 = build_transaction_from(
        Transaction::builder_localnet(Epoch(1))
            .with_inputs(inputs.iter().cloned().map(|i| i.into()))
            .build_and_seal(&PrivateKey::default()),
    );
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx1,
        Decision::Commit,
        5,
        inputs
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        outputs_1.into_iter().chain(outputs_2).collect(),
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx1.clone())
        .await;

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("3")).get_leaf_block();
        if leaf1.height > NodeHeight(30) || leaf2.height > NodeHeight(30) {
            panic!(
                "Not all transaction committed after {}/{} blocks",
                leaf1.height, leaf2.height,
            );
        }
    }

    test.stop();
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;
    test.assert_all_validators_committed(tx1.id());

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_local_inputs_foreign_outputs_abort() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .start()
        .await;

    let inputs = test.create_substates_on_vns(TestVnDestination::Committee(0), 2);
    let outputs = test.build_outputs_for_committee(1, 1);
    let transaction = Transaction::builder_localnet(Epoch(1))
        .with_inputs(inputs.iter().cloned().map(|i| i.into()))
        .build_and_seal(&PrivateKey::default());

    let tx = build_transaction_from(transaction);
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::Committee(0),
        &tx,
        Decision::Commit,
        5,
        inputs
            .clone()
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        outputs.clone(),
    );
    test.send_transaction_to_destination(TestVnDestination::Committee(0), tx.clone())
        .await;

    test.create_execution_at_destination_for_transaction(
        TestVnDestination::Committee(1),
        &tx,
        Decision::Abort(AbortReason::ExecutionFailure),
        5,
        inputs
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        outputs,
    );
    test.send_transaction_to_destination(TestVnDestination::Committee(1), tx.clone())
        .await;

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("3")).get_leaf_block();
        if leaf1.height > NodeHeight(30) || leaf2.height > NodeHeight(30) {
            panic!(
                "Not all transaction committed after {}/{} blocks",
                leaf1.height, leaf2.height,
            );
        }
    }

    test.stop();
    test.assert_all_validators_have_decision(tx.id(), Decision::Abort(AbortReason::ExecutionFailure))
        .await;
    test.assert_all_validators_did_not_commit(tx.id());

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_local_inputs_and_outputs_foreign_outputs() {
    // Transaction involves inputs and outputs for committee 0 and 1, and outputs for committee 2
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .add_committee(2, vec!["5", "6"])
        .start()
        .await;

    let inputs_0 = test.create_substates_on_vns(TestVnDestination::Committee(0), 2);
    let inputs_1 = test.create_substates_on_vns(TestVnDestination::Committee(1), 2);
    let outputs_0 = test.build_outputs_for_committee(0, 5);
    // Output-only committee
    let outputs_2 = test.build_outputs_for_committee(2, 5);

    let tx1 = build_transaction_from(
        Transaction::builder_localnet(Epoch(1))
            .with_inputs(inputs_0.iter().chain(&inputs_1).cloned().map(|i| i.into()))
            .build_and_seal(&PrivateKey::from_canonical_bytes(&[1; 32]).unwrap()),
    );
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx1,
        Decision::Commit,
        5,
        inputs_0
            .into_iter()
            .chain(inputs_1)
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        outputs_0.into_iter().chain(outputs_2).collect(),
    );
    test.send_transaction_to_destination(TestVnDestination::Committee(0), tx1.clone())
        .await;
    test.send_transaction_to_destination(TestVnDestination::Committee(1), tx1.clone())
        .await;
    // Don't send to committee 2 since they are not involved in inputs (simulated mempool behaviour)

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("3")).get_leaf_block();
        if leaf1.height > NodeHeight(30) || leaf2.height > NodeHeight(30) {
            panic!(
                "Not all transaction committed after {}/{} blocks",
                leaf1.height, leaf2.height,
            );
        }
    }

    test.stop();
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;
    test.assert_all_validators_committed(tx1.id());

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_output_conflict_abort() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .start()
        .await;

    let (tx1, inputs) = test.build_transaction(5);
    let mut outputs = test.build_outputs_for_committee(0, 1);
    outputs.extend(test.build_outputs_for_committee(1, 1));
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx1,
        Decision::Commit,
        5,
        inputs
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        outputs.clone(),
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx1.clone())
        .await;

    let inputs = test.create_substates_on_vns(TestVnDestination::All, 1);
    let tx = Transaction::builder_localnet(Epoch(1))
        .with_inputs(inputs.iter().cloned().map(|i| i.into()))
        .build_and_seal(&Default::default());
    let tx2 = build_transaction_from(tx);
    assert_ne!(tx1.id(), tx2.id(), "tx1 and tx2 should be different");
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx2,
        Decision::Commit,
        5,
        inputs
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        outputs,
    );

    let tx_ids = [tx1.id(), tx2.id()];

    test.send_transaction_to_destination(TestVnDestination::All, tx2.clone())
        .await;

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("3")).get_leaf_block();
        if leaf1.height > NodeHeight(30) || leaf2.height > NodeHeight(30) {
            panic!(
                "Not all transaction committed after {}/{} blocks",
                leaf1.height, leaf2.height,
            );
        }
    }

    test.stop();
    // Currently not deterministic (test harness) which transaction will arrive first so we check that one transaction
    // is committed and the other is aborted. TODO: It is also possible that both are aborted.
    let tx1_vn1 = test
        .get_validator(&TestAddress::new("1"))
        .get_transaction_execution(tx_ids[0]);
    let tx2_vn1 = test
        .get_validator(&TestAddress::new("1"))
        .get_transaction_execution(tx_ids[1]);

    let tx1_vn3 = test
        .get_validator(&TestAddress::new("3"))
        .get_transaction_execution(tx_ids[0]);
    let tx2_vn3 = test
        .get_validator(&TestAddress::new("3"))
        .get_transaction_execution(tx_ids[1]);

    assert_eq!(tx1_vn1.decision(), tx1_vn3.decision());
    assert_eq!(tx2_vn1.decision(), tx2_vn3.decision());
    if tx1_vn1.decision().is_commit() {
        test.assert_all_validators_committed(tx_ids[0]);
    } else {
        test.assert_all_validators_did_not_commit(tx_ids[0]);
    }

    if tx2_vn1.decision().is_commit() {
        test.assert_all_validators_committed(tx_ids[1]);
    } else {
        test.assert_all_validators_did_not_commit(tx_ids[1]);
    }

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_shard_inputs_from_previous_outputs() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1", "2"]).start().await;

    let (tx1, _, outputs) = test.send_transaction_to_all(Decision::Commit, 1, 5, 5).await;
    let prev_outputs = outputs
        .iter()
        .map(|output| SubstateRequirement::versioned(output.clone(), 0))
        .collect::<Vec<_>>();

    let tx2 = Transaction::builder_localnet(Epoch(1))
        .with_inputs(prev_outputs.clone())
        .build_and_seal(&Default::default());
    let tx2 = build_transaction_from(tx2.clone());
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx2,
        Decision::Commit,
        5,
        prev_outputs
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        vec![],
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx2.clone())
        .await;

    test.start_epoch(Epoch(1)).await;

    test.wait_for_n_to_be_finalized(2).await;

    let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
    let leaf2 = test.get_validator(&TestAddress::new("2")).get_leaf_block();
    if leaf1.height > NodeHeight(30) || leaf2.height > NodeHeight(30) {
        panic!(
            "Not all transaction committed after {}/{} blocks",
            leaf1.height, leaf2.height,
        );
    }

    test.stop();
    // Assert that the decision matches for all validators. If tx2 is sequenced first, then it will be aborted due to
    // the input not existing
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;
    let decision_tx2 = test
        .get_validator(&TestAddress::new("1"))
        .get_transaction_execution(tx2.id())
        .decision();
    test.assert_all_validators_have_decision(tx2.id(), decision_tx2).await;
    if let Some(reason) = decision_tx2.abort_reason() {
        assert_eq!(reason, AbortReason::OneOrMoreInputsNotFound);
    }

    test.assert_clean_shutdown().await;
    log::info!("total messages sent: {}", test.network().total_messages_sent());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_inputs_from_previous_outputs() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .start()
        .await;

    let (tx1, _, outputs) = test.send_transaction_to_all(Decision::Commit, 1, 5, 2).await;
    let prev_outputs = outputs
        .iter()
        .map(|output| SubstateRequirement::versioned(output.clone(), 0))
        .collect::<Vec<_>>();

    let tx2 = Transaction::builder_localnet(Epoch(1))
        .with_inputs(prev_outputs.clone())
        .build_and_seal(&Default::default());
    let tx2 = build_transaction_from(tx2.clone());
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx2,
        Decision::Commit,
        5,
        prev_outputs
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        vec![],
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx2.clone())
        .await;

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("3")).get_leaf_block();
        if leaf1.height > NodeHeight(30) || leaf2.height > NodeHeight(30) {
            panic!(
                "Not all transaction committed after {}/{} blocks",
                leaf1.height, leaf2.height,
            );
        }
    }

    test.stop();
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;
    test.assert_all_validators_have_decision(tx2.id(), Decision::Abort(AbortReason::OneOrMoreInputsNotFound))
        .await;
    test.assert_all_validators_committed(tx1.id());
    test.assert_all_validators_did_not_commit(tx2.id());

    test.assert_clean_shutdown().await;
    log::info!("total messages sent: {}", test.network().total_messages_sent());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_shard_input_conflict() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1", "2"]).start().await;

    let substate_id = test.create_substates_on_vns(TestVnDestination::All, 1).pop().unwrap();
    // Distinct sealers: the transaction id excludes the seal nonce, so an identical body sealed
    // by the same key would be one transaction, not two conflicting ones.
    let secret1 = PrivateKey::from_canonical_bytes(&[1u8; 32]).unwrap();
    let secret2 = PrivateKey::from_canonical_bytes(&[2u8; 32]).unwrap();

    let tx1 = Transaction::builder_localnet(Epoch(1))
        .add_input(substate_id.clone())
        .build_and_seal(&secret1);
    let tx1 = TransactionRecord::new(tx1);

    let tx2 = Transaction::builder_localnet(Epoch(1))
        .add_input(substate_id.clone())
        .build_and_seal(&secret2);
    let tx2 = TransactionRecord::new(tx2);

    test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx1.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        input_locks: vec![(substate_id.substate_id().clone(), SubstateLockType::Write)],
        new_outputs: vec![],
        validator_fee_withdrawals: vec![],
    })
    .add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx2.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        input_locks: vec![(substate_id.substate_id().clone(), SubstateLockType::Write)],
        new_outputs: vec![],
        validator_fee_withdrawals: vec![],
    });

    test.network()
        .send_transaction(TestVnDestination::All, tx1.clone())
        .await;
    test.network()
        .send_transaction(TestVnDestination::All, tx2.clone())
        .await;

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf1.height > NodeHeight(30) {
            panic!("Not all transaction committed after {} blocks", leaf1.height,);
        }
    }

    let tx1_decision = test
        .get_validator(&TestAddress::new("1"))
        .get_transaction_execution(tx1.id())
        .decision();
    info!("tx1 = {}", tx1.id());
    info!("tx2 = {}", tx2.id());
    if tx1_decision.is_commit() {
        test.assert_all_validators_committed(tx1.id());
        test.assert_all_validators_did_not_commit(tx2.id());
    } else {
        test.assert_all_validators_did_not_commit(tx1.id());
        test.assert_all_validators_committed(tx2.id());
    }

    test.stop();

    test.assert_clean_shutdown().await;
    log::info!("total messages sent: {}", test.network().total_messages_sent());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn foreign_block_distribution() {
    setup_logger();
    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(60))
        .with_message_filter(Box::new(move |from: &TestAddress, to: &TestAddress, msg| {
            if !matches!(msg, HotstuffMessage::ForeignProposalNotification(_)) {
                return true;
            }

            match from.as_str() {
                // We filter out some messages from each node to foreign committees to ensure we sometimes have to
                // rely on other members of the foreign and local committees to receive the foreign proposal.
                "1" => to == "1" || to == "2" || to == "3",
                "4" => to == "4" || to == "5" || to == "6",
                "7" => to == "7" || to == "8" || to == "9",
                _ => true,
            }
        }))
        .add_committee(0, vec!["1", "2", "3"])
        .add_committee(1, vec!["4", "5", "6"])
        .add_committee(2, vec!["7", "8", "9"])
        .start()
        .await;
    for _ in 0..20 {
        test.send_transaction_to_all(Decision::Commit, 1, 5, 1).await;
    }

    test.network().start();
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("4")).get_leaf_block();
        let leaf3 = test.get_validator(&TestAddress::new("7")).get_leaf_block();
        if leaf1.height > NodeHeight(100) || leaf2.height > NodeHeight(100) || leaf3.height > NodeHeight(100) {
            panic!(
                "Not all transaction committed after {}/{}/{} blocks",
                leaf1.height, leaf2.height, leaf3.height
            );
        }
    }

    test.stop();

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    log::info!("total messages filtered: {}", test.network().total_messages_filtered());
    // Each leader sends 3 proposals to the both foreign committees, so 6 messages per leader. 18 in total.
    // assert_eq!(test.network().total_messages_filtered(), 18);
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_shard_unversioned_inputs() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1", "2"]).start().await;
    // First get transaction in the mempool
    let inputs = test.create_substates_on_vns(TestVnDestination::All, 1);
    // Remove versions from inputs to test substate version resolution
    let unversioned_inputs = inputs
        .iter()
        .map(|i| SubstateRequirement::new(i.substate_id().clone(), None));
    let tx = Transaction::builder_localnet(Epoch(1))
        .with_inputs(unversioned_inputs)
        .build_and_seal(&PrivateKey::default());
    let tx = TransactionRecord::new(tx);

    test.send_transaction_to_destination(TestVnDestination::All, tx.clone())
        .await;
    test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        input_locks: inputs
            .into_iter()
            .map(|input| (input.into_substate_id(), SubstateLockType::Write))
            .collect(),
        new_outputs: vec![],
        validator_fee_withdrawals: vec![],
    });

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(10) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    test.stop();
    test.assert_all_validators_committed(tx.id());

    // Assert all LocalOnly
    test.get_validator(&TestAddress::new("1"))
        .state_store
        .with_read_tx(|tx| {
            let leaf = tx.leaf_block_get(Epoch(1))?;
            let mut block = Some(Block::get(tx, &leaf.block_id)?);
            loop {
                block = block.as_ref().unwrap().get_parent(tx).optional()?;
                let Some(b) = block.as_ref() else {
                    break;
                };

                for cmd in b.commands() {
                    assert!(matches!(cmd, Command::LocalOnly(_)));
                }
            }
            Ok::<_, HotStuffError>(())
        })
        .unwrap();

    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_unversioned_input_conflict() {
    // CASE: Tx1 and Tx2 use id1 and id2 as inputs. Comm1 sequences Tx1 and simultaneously Comm2 sequences Tx2.
    // When they exchange substates, they will try to sequence either transaction but will pick up the lock conflict and
    // propose to abort.
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1"])
        .add_committee(1, vec!["3"])
        .start()
        .await;

    let id0 = test
        .create_substates_on_vns(TestVnDestination::Committee(0), 1)
        .pop()
        .unwrap();
    let id1 = test
        .create_substates_on_vns(TestVnDestination::Committee(1), 1)
        .pop()
        .unwrap();

    // Distinct sealers: the transaction id excludes the seal nonce, so an identical body sealed
    // by the same key would be one transaction, not two conflicting ones.
    let tx1 = Transaction::builder_localnet(Epoch(1))
        .add_input(SubstateRequirement::unversioned(id0.substate_id().clone()))
        .add_input(SubstateRequirement::unversioned(id1.substate_id().clone()))
        .build_and_seal(&PrivateKey::from_canonical_bytes(&[1u8; 32]).unwrap());
    let tx1 = TransactionRecord::new(tx1);

    let tx2 = Transaction::builder_localnet(Epoch(1))
        .add_input(SubstateRequirement::unversioned(id0.substate_id().clone()))
        .add_input(SubstateRequirement::unversioned(id1.substate_id().clone()))
        .build_and_seal(&PrivateKey::from_canonical_bytes(&[2u8; 32]).unwrap());
    let tx2 = TransactionRecord::new(tx2);

    test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx1.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        input_locks: vec![
            (id0.substate_id().clone(), SubstateLockType::Write),
            (id1.substate_id().clone(), SubstateLockType::Write),
        ],
        new_outputs: vec![],
        validator_fee_withdrawals: vec![],
    })
    .add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx2.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        input_locks: vec![
            (id0.substate_id().clone(), SubstateLockType::Write),
            (id1.substate_id().clone(), SubstateLockType::Write),
        ],
        new_outputs: vec![],
        validator_fee_withdrawals: vec![],
    });

    // NOTE: we send tx1 to committee 0 and tx2 to committee 1 to loosely ensure that we create the situation this test
    // is testing. If we sent to all, most of the time one or both of the transactions will commit.
    test.network()
        .send_transaction(TestVnDestination::Committee(0), tx1.clone())
        .await;
    test.network()
        .send_transaction(TestVnDestination::Committee(1), tx2.clone())
        .await;

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("3")).get_leaf_block();
        if leaf1.height > NodeHeight(60) && leaf2.height > NodeHeight(60) {
            test.dump_pool_info();
            test.dump_blocks(&TestAddress::new("1"));
            test.dump_blocks(&TestAddress::new("3"));
            panic!(
                "Not all transaction committed after {}/{} blocks",
                leaf1.height, leaf2.height
            );
        }
    }

    test.stop();

    test.assert_all_validators_have_decision(tx1.id(), Decision::Abort(AbortReason::ForeignPledgeInputConflict))
        .await;
    test.assert_all_validators_have_decision(tx2.id(), Decision::Abort(AbortReason::ForeignPledgeInputConflict))
        .await;

    test.assert_clean_shutdown().await;
    log::info!("total messages sent: {}", test.network().total_messages_sent());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_unversioned_input_conflict_delay_prepare() {
    // CASE: Tx1 and Tx2 use id1 as an input, Committee0 sequences Tx1 and simultaneously Committee1 sequences Tx2.
    // Since the id2 and id3 substates are uncommon to the transactions and live in Committee1, Committee1 can lock both
    // transactions. Committee0 will not have yet pledged the value for id1 to Tx1. This allows Committee0 to delay
    // sequencing Tx1 (due to a soft lock conflict) until Tx2 is finalized. The output of Tx2 will be pledged to
    // Tx1. This is a natural consequence (i.e. no special code) of the local substate locks.
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .start()
        .await;

    let id0 = test
        .create_substates_on_vns(TestVnDestination::Committee(0), 1)
        .pop()
        .unwrap();
    let id1 = test
        .create_substates_on_vns(TestVnDestination::Committee(1), 1)
        .pop()
        .unwrap();
    let id2 = test
        .create_substates_on_vns(TestVnDestination::Committee(1), 1)
        .pop()
        .unwrap();

    let tx1 = Transaction::builder_localnet(Epoch(1))
        .add_input(SubstateRequirement::unversioned(id0.substate_id().clone()))
        .add_input(SubstateRequirement::unversioned(id1.substate_id().clone()))
        .build_and_seal(&Default::default());
    let tx1 = TransactionRecord::new(tx1);

    let tx2 = Transaction::builder_localnet(Epoch(1))
        .add_input(SubstateRequirement::unversioned(id0.substate_id().clone()))
        .add_input(SubstateRequirement::unversioned(id2.substate_id().clone()))
        .build_and_seal(&Default::default());
    let tx2 = TransactionRecord::new(tx2);

    test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx1.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        input_locks: vec![
            (id0.substate_id().clone(), SubstateLockType::Write),
            (id1.substate_id().clone(), SubstateLockType::Write),
        ],
        new_outputs: vec![],
        validator_fee_withdrawals: vec![],
    })
    .add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx2.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        input_locks: vec![
            (id0.substate_id().clone(), SubstateLockType::Write),
            (id2.substate_id().clone(), SubstateLockType::Write),
        ],
        new_outputs: vec![],
        validator_fee_withdrawals: vec![],
    });

    test.network()
        .send_transaction(TestVnDestination::Committee(0), tx1.clone())
        .await;
    test.network()
        .send_transaction(TestVnDestination::Committee(1), tx2.clone())
        .await;

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("3")).get_leaf_block();
        if leaf1.height > NodeHeight(60) || leaf2.height > NodeHeight(60) {
            test.dump_pool_info();
            test.dump_blocks(&TestAddress::new("1"));
            test.dump_blocks(&TestAddress::new("3"));
            panic!(
                "Not all transaction committed after {}/{} blocks",
                leaf1.height, leaf2.height
            );
        }
    }

    test.stop();

    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;
    test.assert_all_validators_have_decision(tx2.id(), Decision::Commit)
        .await;

    test.assert_clean_shutdown().await;
    log::info!("total messages sent: {}", test.network().total_messages_sent());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_publish_template() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .add_committee(2, vec!["5", "6"])
        .add_committee(3, vec!["7", "8"])
        .start()
        .await;
    // Create and send publish template transaction
    let inputs = test.create_substates_on_vns(TestVnDestination::All, 1);
    let (sk, pk) = create_key_pair();
    let wasm = load_binary_fixture("state.wasm");
    let expected_binary_hash = hash_template_code(&wasm);
    let tx = Transaction::builder_localnet(Epoch(1))
        .publish_template(wasm)
        .with_inputs(inputs.iter().cloned().map(Into::into))
        .build_and_seal(&sk);
    let tx = TransactionRecord::new(tx);

    test.send_transaction_to_destination(TestVnDestination::All, tx.clone())
        .await;

    let template_id = PublishedTemplateAddress::from_author_and_binary_hash(&pk.to_byte_type(), &expected_binary_hash);
    test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        input_locks: inputs
            .into_iter()
            .map(|input| (input.into_substate_id(), SubstateLockType::Write))
            .collect(),
        new_outputs: vec![SubstateId::Template(template_id)],
        validator_fee_withdrawals: vec![],
    });

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(30) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    test.stop();
    test.assert_all_validators_committed(tx.id());

    // Assert all have the template
    for (addr, vn) in test.validators() {
        let substate_addr = VersionedSubstateId::new(template_id, 0).to_substate_address();
        let template_substate = vn
            .state_store
            .with_read_tx(|tx| SubstateRecord::get(tx, &substate_addr))
            .unwrap_or_else(|e| panic!("Failed to get template substate from {addr}: {e}"));
        let binary_hash = template_substate
            .substate_value
            .unwrap()
            .into_template()
            .expect("Expected template substate")
            .to_binary_hash();
        assert_eq!(binary_hash, expected_binary_hash, "Template binary does not match");
    }

    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_validator_fee_claim() {
    setup_logger();
    let (claim_sk, claim_pk) = create_key_pair_from_seed(100);
    let claim_bytes = claim_pk.to_byte_type();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .set_claim_key(TestVnDestination::All, claim_pk)
        .start()
        .await;
    // Create and send publish template transaction
    let address = derive_fee_pool_address(&claim_bytes, test.num_preshards(), Shard::first()).unwrap();
    let claim_tx = Transaction::builder_localnet(Epoch(1))
        .claim_validator_fees(address)
        .add_input(address)
        .build_and_seal(&claim_sk);
    let claim_tx = TransactionRecord::new(claim_tx);

    test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: claim_tx.transaction().clone(),
        decision: Decision::Commit,
        fee: 1000,
        input_locks: vec![(address.into(), SubstateLockType::Write)],
        new_outputs: vec![],
        validator_fee_withdrawals: vec![ValidatorFeeWithdrawal { address, amount: 500 }],
    });

    // Get some fees
    let (tx1, _, _) = test.send_transaction_to_all(Decision::Commit, 1000, 1, 1).await;
    let (tx2, _, _) = test.send_transaction_to_all(Decision::Commit, 1000, 1, 1).await;

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();

        if test.is_transaction_finalized_at_destination(TestVnDestination::Committee(0), tx1.id()) &&
            test.is_transaction_finalized_at_destination(TestVnDestination::Committee(0), tx2.id())
        {
            break;
        }

        if leaf.height >= NodeHeight(40) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    // Send a claim
    test.send_transaction_to_destination(TestVnDestination::All, claim_tx.clone())
        .await;
    loop {
        test.on_block_committed().await;

        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();

        // This transaction is guaranteed to involve committee 0, so we only check one committee
        if test.is_transaction_finalized_at_destination(TestVnDestination::Committee(0), claim_tx.id()) {
            break;
        }

        if leaf.height >= NodeHeight(80) {
            panic!("Not all transactions committed after {} blocks", leaf.height);
        }
    }

    test.stop();

    // Assert fee pool exists
    let _fee_pool = test
        .get_validator(&TestAddress::new("1"))
        .state_store
        .with_read_tx(|tx| SubstateRecord::get_latest(tx, &address.into()))
        .unwrap();

    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_transaction_epoch_expired() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1"]).start().await;

    let inputs = test.create_substates_on_vns(TestVnDestination::All, 1);
    let tx = build_transaction_from(
        Transaction::builder_localnet(Epoch(1))
            .with_inputs(inputs.iter().cloned().map(|i| i.into()))
            .with_max_epoch(Epoch(0))
            .call_function(Default::default(), "foo", args![])
            .build_and_seal(&PrivateKey::default()),
    );
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx,
        Decision::Commit,
        1,
        inputs
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        vec![],
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx.clone())
        .await;

    // Start at epoch 1, which exceeds max_epoch(0) — transaction should be aborted
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }
        let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf.height >= NodeHeight(10) {
            panic!("Not all transactions finalized after {} blocks", leaf.height);
        }
    }

    test.stop();
    test.assert_all_validators_have_decision(tx.id(), Decision::Abort(AbortReason::EpochExpired))
        .await;

    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_transaction_epoch_expired() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .start()
        .await;

    let inputs = test.create_substates_on_vns(TestVnDestination::Committee(0), 2);
    let outputs = test.build_outputs_for_committee(1, 1);
    let tx = build_transaction_from(
        Transaction::builder_localnet(Epoch(1))
            .with_inputs(inputs.iter().cloned().map(|i| i.into()))
            .with_max_epoch(Epoch(0))
            .call_function(Default::default(), "foo", args![])
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
        outputs,
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx.clone())
        .await;

    // Start at epoch 1, which exceeds max_epoch(0) — the input committee should abort with EpochExpired.
    // The output committee (committee 1) will not process the transaction because the epoch expiry abort
    // prevents execution and therefore no foreign proposal is sent to it.
    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_finalized_at_destination(TestVnDestination::Committee(0), tx.id()) {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf1.height > NodeHeight(30) {
            panic!("Transaction not finalized after {} blocks", leaf1.height,);
        }
    }

    test.stop();
    test.assert_validators_have_decision(
        TestVnDestination::Committee(0),
        tx.id(),
        Decision::Abort(AbortReason::EpochExpired),
    )
    .await;

    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transaction_missing_from_pool_is_resequenced_from_its_record() {
    setup_logger();
    // Drop proposals so the transaction is pooled but no block can carry it yet, which makes the point
    // at which the pool record is dropped deterministic.
    let drop_proposals = Arc::new(AtomicBool::new(true));
    let drop_proposals_f = drop_proposals.clone();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .with_message_filter(Box::new(move |_from, _to, msg| {
            !(drop_proposals_f.load(Ordering::SeqCst) && matches!(msg, HotstuffMessage::Proposal(_)))
        }))
        // Dropping proposals burns a pacemaker round, so shorten the round...
        .modify_consensus_constants(|c| {
            c.pacemaker_block_time = Duration::from_secs(2);
        })
        // ...and allow more than the default budget of one round plus two seconds.
        .with_test_timeout(Duration::from_secs(30))
        .start()
        .await;
    let (tx1, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    test.start_epoch(Epoch(1)).await;
    test.wait_for_pool_count(TestVnDestination::All, 1).await;

    // Drop "2"'s pool record while keeping its transaction record, as a state sync does. "2" holds the
    // transaction, so the missing-transaction request path has nothing to fetch and cannot repair this;
    // without re-sequencing, "2" no-votes every block carrying tx1 and the committee never reaches quorum.
    assert_eq!(test.get_validator(&TestAddress::new("2")).clear_transaction_pool(), 1);

    drop_proposals.store(false, Ordering::SeqCst);

    // The loop only advances on committed blocks but any hotstuff event refreshes the harness timeout,
    // so bound the whole run to keep a stalled committee a test failure.
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            test.on_block_committed().await;

            if test.is_transaction_pool_empty() {
                break;
            }
            let leaf = test.get_validator(&TestAddress::new("1")).get_leaf_block();
            if leaf.height >= NodeHeight(30) {
                panic!("Transaction not committed after {} blocks", leaf.height);
            }
        }
    })
    .await
    .expect("timed out waiting for tx1 to be committed");

    test.stop();
    test.assert_all_validators_committed(tx1.id());
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;
    test.assert_clean_shutdown().await;
}
