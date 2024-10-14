//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! # Consensus tests
//!
//! How to debug the database:
//!
//! Use `Test::builder().debug_sql("/tmp/test{}.db")...` to create a database file for each validator
//! where {} is replaced with the node address.

use std::time::Duration;

use tari_common_types::types::PrivateKey;
use tari_consensus::hotstuff::HotStuffError;
use tari_dan_common_types::{optional::Optional, Epoch, LockIntent, NodeHeight, SubstateRequirement};
use tari_dan_storage::{
    consensus_models::{
        AbortReason,
        BlockId,
        Command,
        Decision,
        SubstateRequirementLockIntent,
        TransactionRecord,
        VersionedSubstateIdLockIntent,
    },
    StateStore,
    StateStoreReadTransaction,
};
use tari_engine_types::commit_result::RejectReason;
use tari_transaction::Transaction;

use crate::support::{
    build_transaction_from,
    logging::setup_logger,
    ExecuteSpec,
    Test,
    TestAddress,
    TestVnDestination,
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

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_committed();

    // Assert all LocalOnly
    test.get_validator(&TestAddress::new("1"))
        .state_store
        .with_read_tx(|tx| {
            let mut block = tx.blocks_get_tip(Epoch(1), test.get_validator(&TestAddress::new("1")).shard_group)?;
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
        .send_transaction_to_all(Decision::Abort(AbortReason::None), 1, 1, 1)
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

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_have_decision(tx1.id(), Decision::Abort(AbortReason::None))
        .await;

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

    test.assert_all_validators_at_same_height().await;
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

    test.assert_all_validators_at_same_height().await;
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn node_requests_missing_transaction_from_local_leader() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1", "2"]).start().await;
    // First get all transactions in the mempool of node "2". We send to "2" because it is the leader for the next
    // block. We could send to "1" but the test would have to wait for the block time to be hit and block 1 to be
    // proposed before node "1" can propose block 2 with all the transactions.
    for _ in 0..10 {
        let (transaction, inputs) = test.build_transaction(Decision::Commit, 5);
        // All VNs will decide the same thing
        test.create_execution_at_destination_for_transaction(
            TestVnDestination::All,
            &transaction,
            inputs
                .into_iter()
                .map(|input| SubstateRequirementLockIntent::write(input.clone(), input.version()))
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
            panic!("Not all transaction committed after {} blocks", committed_height);
        }
    }

    // Check if we clean the missing transactions table in the DB once the transactions are committed
    test.get_validator(&TestAddress::new("2"))
        .state_store
        .with_read_tx(|tx| {
            let mut block_id = BlockId::zero();
            loop {
                let children = tx.blocks_get_all_by_parent(&block_id).unwrap();
                if block_id.is_zero() {
                    break;
                }

                assert_eq!(children.len(), 1);
                for block in children {
                    if block.is_genesis() {
                        continue;
                    }
                    let missing = tx.blocks_get_pending_transactions(block.id()).unwrap();
                    assert!(missing.is_empty());
                    block_id = *block.id();
                }
            }
            Ok::<_, HotStuffError>(())
        })
        .unwrap();

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_committed();
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

    test.send_transaction_to_all(Decision::Commit, 100, 2, 2).await;

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

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_committed();

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

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_committed();

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
    for _ in 0..20 {
        test.send_transaction_to_all(Decision::Commit, 100, 2, 1).await;
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

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_committed();

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

    let (tx1, inputs) = test.build_transaction(Decision::Commit, 5);
    test.send_transaction_to_destination(TestVnDestination::Committee(0), tx1.clone())
        .await;

    // Change the decision on committee 1 to Abort when executing. This test is not technically valid, as all
    // non-byzantine nodes MUST have the same decision given the same pledges. However, this does test that is it not
    // possible for others to COMMIT without all committees agreeing to COMMIT.
    let mut tx2 = tx1.clone();
    tx2.set_abort_reason(RejectReason::ExecutionFailure("Test aborted".to_string()));

    test.create_execution_at_destination_for_transaction(
        TestVnDestination::Committee(0),
        &tx1,
        inputs
            .iter()
            .map(|input| VersionedSubstateIdLockIntent::write(input.clone(), true).into())
            .collect(),
        vec![],
    )
    .create_execution_at_destination_for_transaction(
        TestVnDestination::Committee(1),
        &tx2,
        inputs
            .into_iter()
            .map(|input| VersionedSubstateIdLockIntent::write(input, true).into())
            .collect(),
        vec![],
    );

    test.send_transaction_to_destination(TestVnDestination::Committee(1), tx2.clone())
        .await;

    test.start_epoch(Epoch(1)).await;

    loop {
        test.on_block_committed().await;

        if test.is_transaction_pool_empty() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("2")).get_leaf_block();
        if leaf1.height > NodeHeight(40) || leaf2.height > NodeHeight(40) {
            panic!(
                "Not all transaction committed after {}/{} blocks",
                leaf1.height, leaf2.height,
            );
        }
    }

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_have_decision(tx2.id(), Decision::Abort(AbortReason::None))
        .await;

    test.assert_clean_shutdown().await;
    log::info!("total messages sent: {}", test.network().total_messages_sent());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_local_inputs_foreign_outputs() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .start()
        .await;

    let inputs = test.create_substates_on_vns(TestVnDestination::Committee(0), 2);
    let outputs = test.build_outputs_for_committee(1, 1);

    let tx1 = build_transaction_from(
        Transaction::builder()
            .with_inputs(inputs.iter().cloned().map(|i| i.into()))
            .sign(&PrivateKey::default())
            .build(),
        Decision::Commit,
    );
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx1,
        inputs
            .into_iter()
            .map(|input| VersionedSubstateIdLockIntent::write(input, true).into())
            .collect(),
        outputs,
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

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;
    test.assert_all_validators_committed();

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_local_inputs_and_outputs_foreign_outputs() {
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
        Transaction::builder()
            .with_inputs(inputs_0.iter().chain(&inputs_1).cloned().map(|i| i.into()))
            .sign(&PrivateKey::default())
            .build(),
        Decision::Commit,
    );
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx1,
        inputs_0
            .into_iter()
            .chain(inputs_1)
            .map(|input| VersionedSubstateIdLockIntent::write(input, true).into())
            .collect(),
        outputs_0.into_iter().chain(outputs_2).collect(),
    );
    test.send_transaction_to_destination(TestVnDestination::Committee(0), tx1.clone())
        .await;
    test.send_transaction_to_destination(TestVnDestination::Committee(1), tx1.clone())
        .await;
    // Don't send to committee 2 since they are not involved in inputs

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

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;
    test.assert_all_validators_committed();

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

    let (tx1, inputs) = test.build_transaction(Decision::Commit, 5);
    let mut outputs = test.build_outputs_for_committee(0, 1);
    outputs.extend(test.build_outputs_for_committee(1, 1));
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx1,
        inputs
            .into_iter()
            .map(|input| VersionedSubstateIdLockIntent::write(input, true).into())
            .collect(),
        outputs.clone(),
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx1.clone())
        .await;

    let inputs = test.create_substates_on_vns(TestVnDestination::All, 1);
    let tx = Transaction::builder()
        .with_inputs(inputs.iter().cloned().map(|i| i.into()))
        .sign(&Default::default())
        .build();
    let tx2 = build_transaction_from(tx, Decision::Commit);
    assert_ne!(tx1.id(), tx2.id());
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx2,
        inputs
            .into_iter()
            .map(|input| VersionedSubstateIdLockIntent::write(input, true).into())
            .collect(),
        outputs,
    );
    // Transactions are sorted in the blocks, because we have a "first come first serve" policy for locking objects
    // the "first" will be Committed and the "last" Aborted
    let mut sorted_tx_ids = [tx1.id(), tx2.id()];
    sorted_tx_ids.sort();

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

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_have_decision(sorted_tx_ids[0], Decision::Commit)
        .await;
    test.assert_all_validators_have_decision(sorted_tx_ids[1], Decision::Abort(AbortReason::None))
        .await;
    test.assert_all_validators_committed();

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
        .map(|output| SubstateRequirement::with_version(output.clone(), 0))
        .collect::<Vec<_>>();

    let tx2 = Transaction::builder()
        .with_inputs(prev_outputs.clone())
        .sign(&Default::default())
        .build();
    let tx2 = build_transaction_from(tx2.clone(), Decision::Commit);
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx2,
        prev_outputs
            .into_iter()
            .map(|input| SubstateRequirementLockIntent::write(input, 0))
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

    test.assert_all_validators_at_same_height().await;
    // We do not work out input dependencies when we sequence transactions in blocks. Currently ordering within a block
    // is lexicographical by transaction id, therefore both will only be committed if tx1 happens to be sequenced
    // first.
    if tx1.id() < tx2.id() {
        test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
            .await;
        test.assert_all_validators_have_decision(tx2.id(), Decision::Commit)
            .await;
    } else {
        test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
            .await;
        test.assert_all_validators_have_decision(tx2.id(), Decision::Abort(AbortReason::None))
            .await;
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
        .map(|output| SubstateRequirement::with_version(output.clone(), 0))
        .collect::<Vec<_>>();

    let tx2 = Transaction::builder()
        .with_inputs(prev_outputs.clone())
        .sign(&Default::default())
        .build();
    let tx2 = build_transaction_from(tx2.clone(), Decision::Commit);
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx2,
        prev_outputs
            .into_iter()
            .map(|input| SubstateRequirementLockIntent::write(input, 0))
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

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;
    test.assert_all_validators_have_decision(tx2.id(), Decision::Abort(AbortReason::None))
        .await;
    test.assert_all_validators_committed();

    test.assert_clean_shutdown().await;
    log::info!("total messages sent: {}", test.network().total_messages_sent());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_shard_input_conflict() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1", "2"]).start().await;

    let substate_id = test.create_substates_on_vns(TestVnDestination::All, 1).pop().unwrap();

    let tx1 = Transaction::builder()
        .add_input(substate_id.clone())
        .sign(&Default::default())
        .build();
    let tx1 = TransactionRecord::new(tx1);

    let tx2 = Transaction::builder()
        .add_input(substate_id.clone())
        .sign(&Default::default())
        .build();
    let tx2 = TransactionRecord::new(tx2);

    test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx1.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        inputs: vec![VersionedSubstateIdLockIntent::write(substate_id.clone(), true).into()],
        new_outputs: vec![],
    })
    .add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx2.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        inputs: vec![VersionedSubstateIdLockIntent::write(substate_id, true).into()],
        new_outputs: vec![],
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

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_committed();

    test.assert_clean_shutdown().await;
    log::info!("total messages sent: {}", test.network().total_messages_sent());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn epoch_change() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1", "2"]).start().await;

    test.start_epoch(Epoch(1)).await;
    let mut remaining_txs = 10;

    loop {
        if remaining_txs > 0 {
            test.send_transaction_to_all(Decision::Commit, 1, 5, 1).await;
        }
        remaining_txs -= 1;
        if remaining_txs == 5 {
            test.start_epoch(Epoch(2)).await;
        }

        if remaining_txs <= 0 && test.is_transaction_pool_empty() {
            break;
        }

        let (_, _, epoch, height) = test.on_block_committed().await;
        if height.as_u64() > 1 && epoch == 2u64 {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        if leaf1.height > NodeHeight(30) {
            panic!("Not all transaction committed after {} blocks", leaf1.height,);
        }
    }

    // Assert epoch changed
    test.get_validator(&TestAddress::new("1"))
        .state_store
        .with_read_tx(|tx| {
            let mut block = tx.blocks_get_tip(Epoch(1), test.get_validator(&TestAddress::new("1")).shard_group)?;
            loop {
                block = block.get_parent(tx)?;
                if block.id().is_zero() {
                    break;
                }
                if block.is_epoch_end() {
                    return Ok::<_, HotStuffError>(());
                }
            }

            panic!("No epoch end block found");
        })
        .unwrap();

    test.assert_all_validators_at_same_height().await;
    // test.assert_all_validators_committed();

    test.assert_clean_shutdown().await;
    log::info!("total messages sent: {}", test.network().total_messages_sent());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn leader_failure_node_goes_down() {
    setup_logger();
    let mut test = Test::builder()
        // Allow enough time for leader failures
        .with_test_timeout(Duration::from_secs(60))
        .with_block_time(Duration::from_secs(2))
        .add_committee(0, vec!["1", "2", "3", "4", "5"])
        .start()
        .await;

    let failure_node = TestAddress::new("4");

    for _ in 0..10 {
        test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
    }

    // Take the VN offline - if we do it in the loop below, all transactions may have already been finalized (local
    // only) by committed block 1
    log::info!("😴 {failure_node} is offline");
    test.network()
        .go_offline(TestVnDestination::Address(failure_node.clone()))
        .await;

    test.start_epoch(Epoch(1)).await;

    loop {
        let (_, _, _, committed_height) = test.on_block_committed().await;

        if committed_height == NodeHeight(1) {
            // This allows a few more leader failures to occur
            test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
            test.wait_for_pool_count(TestVnDestination::All, 1).await;
        }

        if test.validators().filter(|vn| vn.address != failure_node).all(|v| {
            let c = v.get_transaction_pool_count();
            log::info!("{} has {} transactions in pool", v.address, c);
            c == 0
        }) {
            break;
        }

        if committed_height > NodeHeight(50) {
            panic!("Not all transaction committed after {} blocks", committed_height);
        }
    }

    test.assert_all_validators_at_same_height_except(&[failure_node.clone()])
        .await;

    test.validators().filter(|vn| vn.address != failure_node).for_each(|v| {
        assert!(v.has_committed_substates(), "Validator {} did not commit", v.address);
    });

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown_except(&[failure_node]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn foreign_block_distribution() {
    setup_logger();
    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(60))
        .with_message_filter(Box::new(move |from: &TestAddress, to: &TestAddress, _| {
            match from.0.as_str() {
                // We filter our message from each leader to the foreign committees. So we will rely on other members of
                // the local committees to send the message to the foreign committee members. And then on
                // the distribution within the foreign committee.
                "1" => *to == TestAddress::new("1") || *to == TestAddress::new("2") || *to == TestAddress::new("3"),
                "4" => *to == TestAddress::new("4") || *to == TestAddress::new("5") || *to == TestAddress::new("6"),
                "7" => *to == TestAddress::new("7") || *to == TestAddress::new("8") || *to == TestAddress::new("9"),
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

    test.assert_all_validators_at_same_height().await;

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
        .map(|i| SubstateRequirement::new(i.substate_id.clone(), None));
    let tx = Transaction::builder()
        .with_inputs(unversioned_inputs)
        .sign(&PrivateKey::default())
        .build();
    let tx = TransactionRecord::new(tx);

    test.send_transaction_to_destination(TestVnDestination::All, tx.clone())
        .await;
    test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        inputs: inputs
            .into_iter()
            .map(|input| VersionedSubstateIdLockIntent::write(input, true).into())
            .collect(),
        new_outputs: vec![],
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

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_committed();

    // Assert all LocalOnly
    test.get_validator(&TestAddress::new("1"))
        .state_store
        .with_read_tx(|tx| {
            let mut block = Some(tx.blocks_get_tip(Epoch(1), test.get_validator(&TestAddress::new("1")).shard_group)?);
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
async fn multi_shard_unversioned_input_conflict() {
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

    let tx1 = Transaction::builder()
        .add_input(SubstateRequirement::unversioned(id0.substate_id().clone()))
        .add_input(SubstateRequirement::unversioned(id1.substate_id().clone()))
        .sign(&Default::default())
        .build();
    let tx1 = TransactionRecord::new(tx1);

    let tx2 = Transaction::builder()
        .add_input(SubstateRequirement::unversioned(id0.substate_id().clone()))
        .add_input(SubstateRequirement::unversioned(id1.substate_id().clone()))
        .sign(&Default::default())
        .build();
    let tx2 = TransactionRecord::new(tx2);

    test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx1.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        inputs: vec![
            VersionedSubstateIdLockIntent::write(id0.clone(), false).into(),
            VersionedSubstateIdLockIntent::write(id1.clone(), false).into(),
        ],
        new_outputs: vec![],
    })
    .add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx2.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        inputs: vec![
            VersionedSubstateIdLockIntent::write(id0, false).into(),
            VersionedSubstateIdLockIntent::write(id1, false).into(),
        ],
        new_outputs: vec![],
    });

    // Transactions are sorted in the blocks, because we have a "first come first serve" policy for locking objects
    // the "first" will be Committed and the "last" Aborted
    let mut sorted_tx_ids = [tx1.id(), tx2.id()];
    sorted_tx_ids.sort();

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
        let leaf2 = test.get_validator(&TestAddress::new("3")).get_leaf_block();
        if leaf1.height > NodeHeight(30) && leaf2.height > NodeHeight(30) {
            panic!(
                "Not all transaction committed after {}/{} blocks",
                leaf1.height, leaf2.height
            );
        }
    }

    test.assert_all_validators_at_same_height().await;
    test.assert_all_validators_committed();
    test.assert_all_validators_have_decision(tx1.id(), Decision::Commit)
        .await;
    test.assert_all_validators_have_decision(tx2.id(), Decision::Commit)
        .await;
    test.get_validator(&TestAddress::new("1"))
        .state_store
        .with_read_tx(|tx| {
            let t1 = tx.transactions_get(tx1.id()).unwrap();
            let t2 = tx.transactions_get(tx2.id()).unwrap();
            let i1 = t1.resolved_inputs().unwrap();
            let i2 = t2.resolved_inputs().unwrap();

            i1.iter().for_each(|i| {
                let other = i2.iter().find(|i2| i2.substate_id() == i.substate_id()).unwrap();
                // Inputs are resolved to the latest UP version, since both consume the same substate, they will use
                // different versions as their inputs
                assert_ne!(i.version(), other.version());
            });
            Ok::<_, HotStuffError>(())
        })
        .unwrap();

    test.assert_clean_shutdown().await;
    log::info!("total messages sent: {}", test.network().total_messages_sent());
}
