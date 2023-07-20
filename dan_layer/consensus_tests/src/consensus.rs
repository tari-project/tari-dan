//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::hotstuff::HotStuffError;
use tari_dan_common_types::NodeHeight;
use tari_dan_storage::{
    consensus_models::{BlockId, Decision},
    StateStore,
    StateStoreReadTransaction,
};

use crate::support::{Test, TestAddress};

// Although these tests will pass with a single thread, we enable multi threaded mode so that any unhandled race
// conditions can be picked up, plus tests run a little quicker.
#[tokio::test(flavor = "multi_thread")]
async fn propose_blocks_with_queued_up_transactions_until_all_committed() {
    let mut test = Test::builder().add_committee(0, vec!["1"]).start().await;
    // First get all transactions in the mempool
    for _ in 0..10 {
        test.send_transaction_to_all(Decision::Commit, 1, 5).await;
    }
    test.wait_until_new_pool_count(10).await;
    test.network().start();

    loop {
        test.on_block_committed().await;

        if test.are_all_transactions_committed() {
            break;
        }
        let leaf = test.get_validator(&TestAddress("1")).get_leaf_block();
        if leaf.height > NodeHeight(20) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    test.assert_all_validators_at_same_height().await;
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn node_requests_missing_transaction_from_local_leader() {
    let mut test = Test::builder().add_committee(0, vec!["1", "2"]).start().await;
    // First get all transactions in the mempool of node "1"
    for _ in 0..10 {
        test.send_transaction_to(&TestAddress("1"), Decision::Commit, 1, 5)
            .await;
    }
    test.wait_until_new_pool_count_for_vn(10, TestAddress("1")).await;
    test.network().start();
    loop {
        test.on_block_committed().await;

        if test.are_all_transactions_committed() {
            break;
        }
        let leaf = test.get_validator(&TestAddress("1")).get_leaf_block();
        if leaf.height > NodeHeight(10) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    // Check if we clean the missing transactions table in the DB once the transactions are committed
    test.get_validator(&TestAddress("2"))
        .state_store
        .with_read_tx(|tx| {
            let mut block_id = BlockId::genesis();
            while let Ok(block) = tx.blocks_get_by_parent(&block_id) {
                assert!(tx.blocks_get_missing_transactions(block.id()).is_err());
                block_id = *block.id();
            }
            Ok::<_, HotStuffError>(())
        })
        .unwrap();

    test.assert_all_validators_at_same_height().await;
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn propose_blocks_with_new_transactions_until_all_committed() {
    let mut test = Test::builder().add_committee(0, vec!["1"]).start().await;
    let mut remaining_txs = 10;
    test.network().start();
    loop {
        if remaining_txs > 0 {
            test.send_transaction_to_all(Decision::Commit, 1, 5).await;
        }
        remaining_txs -= 1;
        test.on_block_committed().await;

        if test.are_all_transactions_committed() {
            break;
        }
        let leaf = test.get_validator(&TestAddress("1")).get_leaf_block();
        if leaf.height > NodeHeight(20) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    test.assert_all_validators_at_same_height().await;
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_validator_propose_blocks_with_new_transactions_until_all_committed() {
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2", "3", "4", "5"])
        .start()
        .await;
    let mut remaining_txs = 10u32;
    test.network().start();
    loop {
        if remaining_txs > 0 {
            test.send_transaction_to_all(Decision::Commit, 1, 5).await;
        }
        test.on_block_committed().await;
        remaining_txs = remaining_txs.saturating_sub(1);

        if remaining_txs == 0 && test.are_all_transactions_committed() {
            break;
        }
        let leaf = test.get_validator(&TestAddress("1")).get_leaf_block();
        if leaf.height > NodeHeight(20) {
            panic!("Not all transaction committed after {} blocks", leaf.height);
        }
    }

    test.assert_all_validators_at_same_height().await;

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_shard_propose_blocks_with_new_transactions_until_all_committed() {
    let mut test = Test::builder()
        .add_committee(0, vec!["1", "2", "3"])
        .add_committee(1, vec!["4", "5", "6"])
        .add_committee(2, vec!["7", "8", "9"])
        .start()
        .await;
    for _ in 0..20 {
        test.send_transaction_to_all(Decision::Commit, 1, 5).await;
    }

    test.wait_all_have_at_least_n_new_transactions_in_pool(20).await;
    test.network().start();

    loop {
        test.on_block_committed().await;

        if test.are_all_transactions_committed() {
            break;
        }

        let leaf1 = test.get_validator(&TestAddress("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress("4")).get_leaf_block();
        let leaf3 = test.get_validator(&TestAddress("7")).get_leaf_block();
        if leaf1.height > NodeHeight(20) || leaf2.height > NodeHeight(20) || leaf3.height > NodeHeight(20) {
            panic!(
                "Not all transaction committed after {}/{}/{} blocks",
                leaf1.height, leaf2.height, leaf3.height
            );
        }
    }

    test.assert_all_validators_at_same_height().await;

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}
