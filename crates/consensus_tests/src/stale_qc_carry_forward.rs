//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Edge-case test: stale-QC carry-forward after a vote-loss round.
//!
//! Non-malicious scenario that a correct HotStuff implementation must recover from. Votes for an
//! early block at height H are dropped by the network — quorum can never form for that block, the
//! pacemaker eventually fires a TC, and the next leader proposes at H+1 carrying the *older*
//! HighQC (one that does not justify H) together with the TC. This exercises the same code paths
//! the live-network stall surfaced (proposer's `accumulated_data` and state-anchor initialization
//! when filling a timeout gap with dummies), but reaches them via realistic message-timing
//! conditions rather than copied state. Once the filter has done its job, votes flow normally and
//! consensus must finalise the pending transactions.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tari_consensus::messages::HotstuffMessage;
use tari_consensus_types::Decision;
use tari_ootle_common_types::{Epoch, NodeHeight};
use tari_ootle_storage::{StateStore, consensus_models::Block};

use crate::support::{Test, TestVnDestination, logging::setup_logger};

/// All votes for the first real block are dropped. With f=1 in a 4-node committee, that means no
/// QC can form for height 1. The pacemaker fires a TC, the next leader proposes at height 2 with
/// HighQC still pointing at genesis plus the TC for height 1 — i.e. a stale-QC carry-forward.
/// After the round, the filter stops triggering and consensus must converge and finalise the
/// pending transaction.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_round_votes_dropped_recovers_via_stale_qc_carry_forward() {
    setup_logger();

    let votes_dropped = Arc::new(AtomicUsize::new(0));
    let counter = votes_dropped.clone();
    let target_height = NodeHeight(1);

    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(90))
        .modify_consensus_constants(|c| {
            // Tight pacemaker so the TC fires quickly inside the test budget.
            c.pacemaker_block_time = Duration::from_secs(3);
            c.missed_proposal_suspend_threshold = 10;
        })
        .with_message_filter(Box::new(move |_from, _to, msg| {
            // Return false to drop. Drop every Vote message whose vote height is the target.
            if let HotstuffMessage::Vote(v) = msg &&
                v.vote.block_height == target_height
            {
                counter.fetch_add(1, Ordering::SeqCst);
                return false;
            }
            true
        }))
        .add_committee(0, vec!["1", "2", "3", "4"])
        .start()
        .await;

    let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
    let tx_id = *tx.id();

    test.start_epoch(Epoch(1)).await;

    // Drive the test loop until the transaction is finalised everywhere AND we've witnessed a
    // real (non-dummy) block commit past the dropped-vote height — that's the only proof the
    // recovery path actually carried forward, not just the dummy filler the chain-commit
    // cascade walks back to.
    //
    // Why both conditions: the cascade emits a `BlockCommitted` event for the dummy at
    // target_height (height matches but it's a filler) before the event for the real recovery
    // block at target_height+1. Both publishes happen inside the same storage write
    // transaction, so substates become visible roughly together across the committee. Without
    // the dummy filter, `has_committed_substates` can return true for everyone while the loop
    // has only consumed the dummy events, leaving `saw_past_target` false even though recovery
    // succeeded.
    let mut saw_past_target = false;
    loop {
        let (address, block_id, _, committed_height) = test.on_block_committed().await;
        if committed_height > target_height {
            let is_dummy = test
                .get_validator(&address)
                .state_store()
                .with_read_tx(|tx| Block::get(tx, &block_id))
                .expect("block from BlockCommitted event must exist in the validator's store")
                .is_dummy();
            if !is_dummy {
                saw_past_target = true;
            }
        }
        if saw_past_target && test.validators_iter().all(|v| v.has_committed_substates(&tx_id)) {
            break;
        }
        if committed_height > NodeHeight(50) {
            panic!(
                "transaction {tx_id} not finalised after {} blocks (votes dropped at height {target_height}: {})",
                committed_height,
                votes_dropped.load(Ordering::SeqCst)
            );
        }
    }

    assert!(
        votes_dropped.load(Ordering::SeqCst) > 0,
        "test premise: at least one vote at height {target_height} must have been dropped"
    );
    assert!(
        saw_past_target,
        "consensus must advance past the dropped-vote height for the recovery path to be exercised"
    );

    // Sanity: pool drains everywhere.
    test.wait_for_pool_count(TestVnDestination::All, 0).await;
    test.stop();
    test.assert_clean_shutdown().await;
}
