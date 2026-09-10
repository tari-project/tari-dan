//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use tari_consensus_types::Decision;
use tari_ootle_common_types::{Epoch, NodeHeight};

use crate::support::{Test, TestAddress, TestVnDestination, logging::setup_logger};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_shard_node_goes_down() {
    setup_logger();
    let mut test = Test::builder()
        // Allow enough time for leader failures
        .with_test_timeout(Duration::from_secs(60))
        .modify_consensus_constants(|config_mut| {
            config_mut.missed_proposal_suspend_threshold = 10;
            config_mut.pacemaker_block_time = Duration::from_secs(5);
        })
        .add_committee(0, vec!["1", "2", "3", "4", "5"])
        .start()
        .await;

    let failure_node = TestAddress::new("4");

    let mut tx_ids = Vec::with_capacity(10);
    for _ in 0..10 {
        let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
        tx_ids.push(*tx.id());
    }

    // Take the VN offline - if we do it in the loop below, all transactions may have already been finalized (local
    // only) by committed block 1
    log::info!("😴 {failure_node} is offline");
    test.network().go_offline(failure_node.clone()).await;

    test.start_epoch(Epoch(1)).await;

    loop {
        let (_, _, _, committed_height) = test.on_block_committed().await;

        if committed_height == NodeHeight(1) {
            // This allows a few more leader failures to occur
            let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
            test.wait_for_transaction_seen(TestVnDestination::All, tx.id()).await;
        }

        if test.validators_iter().filter(|vn| vn.address != failure_node).all(|v| {
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

    test.stop();

    test.validators_iter()
        .filter(|vn| vn.address != failure_node)
        .for_each(|v| {
            tx_ids.iter().for_each(|tx_id| {
                assert!(
                    v.has_committed_substates(tx_id),
                    "Validator {} did not commit",
                    v.address
                );
            });
        });

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown_except(&[failure_node]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_shard_neighbour_nodes_go_down() {
    // "neighbour" meaning next to each other in the leader order
    setup_logger();
    let mut test = Test::builder()
        // Allow enough time for leader failures
        .with_test_timeout(Duration::from_secs(60))
        .modify_consensus_constants(|config_mut| {
            config_mut.missed_proposal_suspend_threshold = 10;
            config_mut.pacemaker_block_time = Duration::from_secs(5);
        })
        // For f = 2 we need 7 nodes
        .add_committee(0, vec!["1", "2", "3", "4", "5", "6", "7"])
        .start()
        .await;

    let failure_node1 = TestAddress::new("4");
    let failure_node2 = TestAddress::new("5");

    let mut tx_ids = Vec::with_capacity(10);
    for _ in 0..10 {
        let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
        tx_ids.push(*tx.id());
    }

    // Take the VN offline - if we do it in the loop below, all transactions may have already been finalized (local
    // only) by committed block 1
    log::info!("😴 {failure_node1} is offline");
    log::info!("😴 {failure_node2} is offline");
    test.network().go_offline(failure_node1.clone()).await;
    test.network().go_offline(failure_node2.clone()).await;

    test.start_epoch(Epoch(1)).await;

    loop {
        let (_, _, _, committed_height) = test.on_block_committed().await;

        if committed_height == NodeHeight(1) {
            // This allows a few more leader failures to occur
            let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
            test.wait_for_transaction_seen(TestVnDestination::All, tx.id()).await;
        }

        if test
            .validators_iter()
            .filter(|vn| vn.address != failure_node1 && vn.address != failure_node2)
            .all(|v| {
                let c = v.get_transaction_pool_count();
                log::info!("{} has {} transactions in pool", v.address, c);
                c == 0
            })
        {
            break;
        }

        if committed_height > NodeHeight(50) {
            panic!("Not all transaction committed after {} blocks", committed_height);
        }
    }

    test.stop();

    test.validators_iter()
        .filter(|vn| vn.address != failure_node1 && vn.address != failure_node2)
        .for_each(|v| {
            tx_ids.iter().for_each(|tx_id| {
                assert!(
                    v.has_committed_substates(tx_id),
                    "Validator {} did not commit",
                    v.address
                );
            });
        });

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown_except(&[failure_node1]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multi_shard_node_goes_down() {
    // Although leader failure does not generally affect the other shards, we test that the foreign proposal commit
    // proof still validates with dummy blocks included
    setup_logger();
    let mut test = Test::builder()
        // Allow enough time for leader failures
        .with_test_timeout(Duration::from_secs(60))
        .modify_consensus_constants(|config_mut| {
            config_mut.missed_proposal_suspend_threshold = 10;
            config_mut.pacemaker_block_time = Duration::from_secs(5);
        })
        .add_committee(0, vec!["1", "2", "3", "4", "5"])
        .add_committee(1, vec!["6", "7"])
        .start()
        .await;

    let failure_node = TestAddress::new("4");
    let failure_group_nodes: Vec<TestAddress> = vec!["1", "2", "3", "5"].into_iter().map(TestAddress::new).collect();

    let mut tx_ids = Vec::with_capacity(10);
    for _ in 0..10 {
        let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
        tx_ids.push(*tx.id());
    }

    // Take the VN offline - if we do it in the loop below, all transactions may have already been finalized (local
    // only) by committed block 1
    log::info!("😴 {failure_node} is offline");
    test.network().go_offline(failure_node.clone()).await;

    test.start_epoch(Epoch(1)).await;

    // Track committed height only for the shard group with the offline node, since the other shard group races ahead
    // with empty blocks and would trigger the height limit prematurely.
    let mut failure_group_height = NodeHeight(0);
    let mut sent_extra_tx = false;
    loop {
        let (address, _, _, committed_height) = test.on_block_committed().await;

        if failure_group_nodes.contains(&address) {
            failure_group_height = committed_height;
        }

        if committed_height == NodeHeight(1) && !sent_extra_tx {
            sent_extra_tx = true;
            // This allows a few more leader failures to occur
            let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
            test.wait_for_transaction_seen(TestVnDestination::All, tx.id()).await;
        }

        if test.validators_iter().filter(|vn| vn.address != failure_node).all(|v| {
            let c = v.get_transaction_pool_count();
            log::info!("{} has {} transactions in pool", v.address, c);
            c == 0
        }) {
            break;
        }

        if failure_group_height > NodeHeight(50) {
            panic!(
                "Not all transaction committed after {} blocks in failure shard group",
                failure_group_height
            );
        }
    }

    test.stop();

    // TODO: assert something - transactions are not guaranteed to involve all shard groups
    // test.validators_iter()
    //     .filter(|vn| vn.address != failure_node)
    //     .for_each(|v| {
    //         tx_ids.iter().for_each(|tx_id| {
    //             assert!(
    //                 v.has_committed_substates(tx_id),
    //                 "Validator {} did not commit",
    //                 v.address
    //             );
    //         });
    //     });

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown_except(&[failure_node]).await;
}

/// Regression test: when the first leader in a new epoch fails before any block is committed,
/// the HighQC still justifies the zero block. The proposer must use the epoch genesis block
/// (not the global zero block) when calculating dummy blocks, otherwise the dummy chain
/// diverges from what validators expect and all proposals are permanently rejected.
///
/// If the bug is present, no block will ever be committed and the test will timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_leader_failure_at_epoch_start_recovers() {
    setup_logger();
    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(60))
        .modify_consensus_constants(|config_mut| {
            config_mut.missed_proposal_suspend_threshold = 10;
            config_mut.pacemaker_block_time = Duration::from_secs(5);
        })
        .add_committee(0, vec!["1", "2", "3", "4", "5"])
        .start()
        .await;

    // Node "1" is at committee position 0, so it is the leader for view 0 (the first view in
    // the epoch). Taking it offline ensures no blocks are committed before timeouts trigger,
    // keeping the HighQC at the zero block QC.
    let failure_node = TestAddress::new("1");
    log::info!("😴 {failure_node} (first leader) is offline");
    test.network().go_offline(failure_node.clone()).await;

    test.start_epoch(Epoch(1)).await;

    // Wait for at least one block to be committed. This proves that the remaining validators
    // successfully proposed with dummy blocks from the epoch genesis.
    test.on_block_committed().await;

    test.stop();
    test.assert_clean_shutdown_except(&[failure_node]).await;
}

/// Regression test for stale `pending_stage` after orphaned blocks.
///
/// When a validator votes for a block that later ends up on a dead branch (no QC formed),
/// the transaction in that block must still be correctly re-proposed and finalized by the next leader.
/// Previously, an eager write of `pending_stage` to the base transaction pool record caused a permanent
/// "Stage disagreement" because the stale stage was never cleaned up after the block was orphaned.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_shard_transaction_finalizes_after_orphaned_block() {
    setup_logger();

    // Drop the first round of votes to prevent QC formation for the first proposed block.
    // This creates an orphaned block: validators receive and evaluate the proposal (saving state updates
    // and setting pending_stage), but the block never gets a QC.
    // With n=5, each block can receive up to 5 votes (including a newview+vote combo).
    // Dropping the first 5 vote-bearing messages ensures the first block's QC cannot form.
    let dropped_vote_count = std::sync::Arc::new(AtomicUsize::new(0));
    let dropped_vote_count_clone = dropped_vote_count.clone();
    let num_votes_to_drop = 5;

    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(60))
        .modify_consensus_constants(|config_mut| {
            config_mut.missed_proposal_suspend_threshold = 10;
            config_mut.pacemaker_block_time = Duration::from_secs(5);
        })
        .add_committee(0, vec!["1", "2", "3", "4", "5"])
        .with_message_filter(Box::new(move |_from, _to, msg| {
            // Drop Vote and NewView (which carry votes) messages for the first round
            let is_vote_bearing = matches!(
                msg,
                tari_consensus::messages::HotstuffMessage::Vote(_) |
                    tari_consensus::messages::HotstuffMessage::NewView(_)
            );
            if is_vote_bearing {
                let count = dropped_vote_count_clone.fetch_add(1, Ordering::SeqCst);
                if count < num_votes_to_drop {
                    log::info!("🔇 Dropping vote-bearing message {}/{num_votes_to_drop}", count + 1);
                    return false;
                }
            }
            true
        }))
        .start()
        .await;

    let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
    let tx_id = *tx.id();

    test.start_epoch(Epoch(1)).await;

    loop {
        let (_, _, _, committed_height) = test.on_block_committed().await;

        if test.validators_iter().all(|v| {
            let c = v.get_transaction_pool_count();
            log::info!("{} has {} transactions in pool", v.address, c);
            c == 0
        }) {
            break;
        }

        if committed_height > NodeHeight(20) {
            panic!(
                "Transaction not finalized after {committed_height} blocks. This likely indicates a stage \
                 disagreement caused by stale pending_stage from the orphaned block."
            );
        }
    }

    test.stop();

    test.validators_iter().for_each(|v| {
        assert!(
            v.has_committed_substates(&tx_id),
            "Validator {} did not commit transaction after orphaned block recovery",
            v.address
        );
    });

    // Verify that votes were actually dropped (the orphan condition was triggered)
    let total_dropped = dropped_vote_count.load(Ordering::SeqCst);
    assert!(
        total_dropped >= num_votes_to_drop,
        "Expected at least {num_votes_to_drop} votes to be dropped, but only {total_dropped} were seen"
    );

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_shard_node_goes_down_and_catches_up() {
    setup_logger();
    let mut test = Test::builder()
        // Allow enough time for leader failures
        .with_test_timeout(Duration::from_secs(60))
        .modify_consensus_constants(|constants_mut| {
            constants_mut.missed_proposal_suspend_threshold = 10;
            constants_mut.pacemaker_block_time = Duration::from_secs(5);
        })
        .add_committee(0, vec!["1", "2", "3", "4", "5"])
        .start()
        .await;

    let failure_node = TestAddress::new("4");

    let mut tx_ids = Vec::with_capacity(12);
    for _ in 0..10 {
        let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
        tx_ids.push(*tx.id());
    }

    test.start_epoch(Epoch(1)).await;
    let epoch_start = Instant::now();
    let mut is_back_online = false;
    let mut had_gone_offline = false;

    loop {
        let (_, _, _, committed_height) = test.on_block_committed().await;

        if !had_gone_offline && epoch_start.elapsed() >= Duration::from_secs(2) {
            log::info!("😴 {failure_node} is offline");
            test.network().go_offline(failure_node.clone()).await;
            let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
            tx_ids.push(*tx.id());
            had_gone_offline = true;
        }

        if !is_back_online && epoch_start.elapsed() >= Duration::from_secs(13) {
            log::info!("🚀 {failure_node} is online again");
            test.network().go_online(&failure_node).await;
            let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
            is_back_online = true;
            tx_ids.push(*tx.id());
        }

        if committed_height == NodeHeight(1) {
            // This allows a few more leader failures to occur
            let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
            test.wait_for_transaction_seen(TestVnDestination::All, tx.id()).await;
        }

        if is_back_online &&
            test.validators_iter()
                .all(|v| tx_ids.iter().all(|tx_id| v.has_committed_substates(tx_id)))
        {
            break;
        }

        if committed_height > NodeHeight(50) {
            panic!("Not all transaction committed after {} blocks", committed_height);
        }
    }

    test.stop();

    log::info!("total messages sent: {}", test.network().total_messages_sent());
    test.assert_clean_shutdown_except(&[failure_node]).await;
}
