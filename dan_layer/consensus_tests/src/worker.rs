//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::Epoch;
use tari_dan_storage::{
    consensus_models::{Decision, TransactionPool},
    StateStore,
    StateStoreReadTransaction,
};

use crate::support::Test;

// Although these tests will pass with a single thread, we enable multi threaded mode so that any unhandled race
// conditions can be picked up.
#[tokio::test(flavor = "multi_thread")]
async fn propose_blocks_with_queued_up_transactions_until_all_committed() {
    let mut test = Test::builder().with_addresses(vec!["1"]).start().await;
    // First get all transactions in the mempool
    for _ in 0..10 {
        test.send_all_transaction(Decision::Accept, 1).await;
    }
    test.wait_until_new_pool_count(10).await;
    let mut round = 0;
    loop {
        test.do_hotstuff_round(round).await;

        if test.is_all_transactions_committed() {
            break;
        }

        if round == 20 {
            panic!("Took more than 20 rounds to commit all transactions");
        }
        round += 1;
    }

    test.with_all_validators(|vn| {
        let mut tx = vn.state_store.create_read_tx().unwrap();
        let leaf = tx.leaf_block_get(Epoch(0)).unwrap();
        assert_eq!(leaf.height.as_u64(), 3); // 1 block per phase
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn propose_blocks_with_new_transactions_until_all_committed() {
    let mut test = Test::builder().with_addresses(vec!["1"]).start().await;
    let mut remaining_txs = 10;
    let mut round = 0;
    loop {
        if remaining_txs > 0 {
            test.send_all_transaction(Decision::Accept, 1).await;
        }
        remaining_txs -= 1;

        test.do_hotstuff_round(round).await;

        if test.is_all_transactions_committed() {
            break;
        }
        if round == 20 {
            panic!("Took more than 20 rounds to commit all transactions");
        }
        round += 1;
    }

    test.with_all_validators(|vn| {
        let mut tx = vn.state_store.create_read_tx().unwrap();
        let leaf = tx.leaf_block_get(Epoch(0)).unwrap();
        // The number of blocks needed depends on if new each transaction makes it into the new pool before the next
        // proposal There should be at least 10 because we perform a round after sending each. At most 15(or maybe 14),
        // because we stop sending transactions after round 10 but still continue running consensus.
        assert!(leaf.height.as_u64() > 10);
        assert!(leaf.height.as_u64() <= 15);
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_validator_propose_blocks_with_new_transactions_until_all_committed() {
    let mut test = Test::builder()
        .with_addresses(vec!["1", "2", "3", "4", "5"])
        .start()
        .await;
    let mut remaining_txs = 10u32;
    let mut round = 0;
    loop {
        if remaining_txs > 0 {
            test.send_all_transaction(Decision::Accept, 1).await;

            // In the consensus implementation, we expect all transactions to be present
            test.wait_all_have_at_least_n_transactions_in_pool(1, TransactionPool::New)
                .await;
        }
        remaining_txs = remaining_txs.saturating_sub(1);

        test.do_hotstuff_round(round).await;
        if remaining_txs == 0 && test.is_all_transactions_committed() {
            break;
        }
        if round == 20 {
            panic!("Took more than 20 rounds to commit all transactions");
        }
        round += 1;
    }

    test.with_all_validators(|vn| {
        let mut tx = vn.state_store.create_read_tx().unwrap();
        let leaf = tx.leaf_block_get(Epoch(0)).unwrap();
        // The number of blocks needed depends on if new each transaction makes it into the new pool before the next
        // proposal There should be at least 10 because we perform a round after sending each. At most 15(maybe 14, but
        // I think there's a chance the tx is only picked up on the 11th proposal), because we stop
        // sending transactions after round 10 but still continue running consensus.
        assert!(leaf.height.as_u64() > 10);
        assert!(leaf.height.as_u64() <= 15);
    });

    log::info!("total messages sent: {}", test.total_messages_sent());
}
