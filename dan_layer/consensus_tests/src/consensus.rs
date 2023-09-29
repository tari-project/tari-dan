//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! # Consensus tests
//!
//! How to debug the database:
//!
//! Use `Test::builder().debug_sql("/tmp/test{}.db")...` to create a database file for each validator
//! where {} is replaced with the node address.

use std::time::Duration;

use tari_consensus::hotstuff::HotStuffError;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{BlockId, Decision},
    StateStore,
    StateStoreReadTransaction,
};
use tari_transaction::Transaction;

use crate::support::{
    build_transaction,
    build_transaction_from,
    change_decision,
    logging::setup_logger,
    Test,
    TestAddress,
    TestNetworkDestination,
};

// Although these tests will pass with a single thread, we enable multi threaded mode so that any unhandled race
// conditions can be picked up, plus tests run a little quicker.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_transaction() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1"]).start().await;
    // First get transaction in the mempool
    test.send_transaction_to_all(Decision::Commit, 1, 1).await;
    test.wait_until_new_pool_count(1).await;
    test.start_epoch(Epoch(0));

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
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn propose_blocks_with_queued_up_transactions_until_all_committed() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1"]).start().await;
    // First get all transactions in the mempool
    for _ in 0..10 {
        test.send_transaction_to_all(Decision::Commit, 1, 5).await;
    }
    test.wait_until_new_pool_count(10).await;
    test.start_epoch(Epoch(0));

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
async fn node_requests_missing_transaction_from_local_leader() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1", "2"]).start().await;
    // First get all transactions in the mempool of node "2". We send to "2" because it is the leader for the next
    // block. We could send to "1" but the test would have to wait for the block time to be hit and block 1 to be
    // proposed before node "1" can propose block 2 with all the transactions.
    for _ in 0..10 {
        test.send_transaction_to(&TestAddress::new("2"), Decision::Commit, 1, 5)
            .await;
    }
    test.wait_until_new_pool_count_for_vn(10, TestAddress::new("2")).await;
    test.start_epoch(Epoch(0));
    loop {
        let (_, committed_height) = test.on_block_committed().await;

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
            let mut block_id = BlockId::genesis();
            loop {
                let children = tx.blocks_get_all_by_parent(&block_id).unwrap();
                if children.is_empty() {
                    break;
                }
                if !block_id.is_genesis() {
                    assert_eq!(children.len(), 1);
                }
                for block in children {
                    if block.id().is_genesis() {
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
async fn propose_blocks_with_new_transactions_until_all_committed() {
    setup_logger();
    let mut test = Test::builder().add_committee(0, vec!["1"]).start().await;
    let mut remaining_txs = 10;
    test.start_epoch(Epoch(0));
    loop {
        if remaining_txs > 0 {
            test.send_transaction_to_all(Decision::Commit, 1, 5).await;
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
async fn multi_validator_propose_blocks_with_new_transactions_until_all_committed() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2", "3", "4", "5"])
        .start()
        .await;
    let mut remaining_txs = 10u32;
    test.start_epoch(Epoch(0));
    loop {
        if remaining_txs > 0 {
            test.send_transaction_to_all(Decision::Commit, 1, 5).await;
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
        test.send_transaction_to_all(Decision::Commit, 100, 5).await;
    }

    test.wait_all_have_at_least_n_new_transactions_in_pool(20).await;
    test.start_epoch(Epoch(0));

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
async fn foreign_shard_decides_to_abort() {
    setup_logger();
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "3", "4"])
        .add_committee(1, vec!["2", "5", "6"])
        .start()
        .await;

    let tx1 = build_transaction(Decision::Commit, 1, 5, 2);
    test.network()
        .send_transaction(TestNetworkDestination::Bucket(0), tx1.clone())
        .await;
    let tx2 = change_decision(tx1.clone(), Decision::Abort);
    test.network()
        .send_transaction(TestNetworkDestination::Bucket(1), tx2.clone())
        .await;
    assert_eq!(tx1.id(), tx2.id());

    test.wait_all_have_at_least_n_new_transactions_in_pool(1).await;
    test.start_epoch(Epoch(0));

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
    test.assert_all_validators_have_decision(tx1.id(), Decision::Abort)
        .await;
    test.assert_all_validators_did_not_commit();

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn leader_failure_output_conflict() {
    setup_logger();
    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(60))
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .start()
        .await;

    let tx1 = build_transaction(Decision::Commit, 1, 5, 2);
    let resulting_outputs = tx1.resulting_outputs().to_vec();
    test.network()
        .send_transaction(TestNetworkDestination::All, tx1.clone())
        .await;

    let tx = Transaction::builder().sign(&Default::default()).build();
    let tx2 = build_transaction_from(tx, Decision::Commit, 1, resulting_outputs);
    assert_ne!(tx1.id(), tx2.id());
    // Transactions are sorted in the blocks, because we have a "first come first serve" policy for locking objects
    // the "first" will be Committed and the "last" Aborted
    let mut sorted_tx_ids = [tx1.id(), tx2.id()];
    sorted_tx_ids.sort();

    test.network()
        .send_transaction(TestNetworkDestination::All, tx2.clone())
        .await;

    test.wait_all_have_at_least_n_new_transactions_in_pool(2).await;
    test.start_epoch(Epoch(0));

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
    test.assert_all_validators_have_decision(sorted_tx_ids[0], Decision::Commit)
        .await;
    test.assert_all_validators_have_decision(sorted_tx_ids[1], Decision::Abort)
        .await;
    test.assert_all_validators_committed();

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn leader_failure_node_goes_down() {
    setup_logger();
    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(60))
        .add_committee(0, vec!["1", "2", "3", "4", "5"])
        .start()
        .await;

    let failure_node = TestAddress::new("2");

    for _ in 0..10 {
        test.send_transaction_to_all(Decision::Commit, 1, 2).await;
    }
    test.wait_all_have_at_least_n_new_transactions_in_pool(10).await;
    test.start_epoch(Epoch(0));

    loop {
        let (_, committed_height) = test.on_block_committed().await;

        if committed_height == NodeHeight(1) {
            log::info!("😴 Node 2 goes offline");
            test.network()
                .go_offline(TestNetworkDestination::Address(failure_node.clone()))
                .await;
        }

        if test.validators().filter(|vn| vn.address != failure_node).all(|v| {
            let c = v.get_transaction_pool_count();
            log::info!("{} has {} transactions in pool", v.address, c);
            c == 0
        }) {
            break;
        }

        if committed_height > NodeHeight(20) {
            panic!("Not all transaction committed after {} blocks", committed_height);
        }
    }

    test.assert_all_validators_at_same_height_except(&[failure_node.clone()])
        .await;

    assert!(test
        .validators()
        .filter(|vn| vn.address != failure_node)
        .all(|v| v.state_manager().is_committed()));

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}
