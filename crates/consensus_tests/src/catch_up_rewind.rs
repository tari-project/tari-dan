//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Regression test: catch-up view-rewind deadlock.
//!
//! A node can hold a **stored** block above its highest certified block (`HighestSeenBlock >
//! HighPC`) — this is the normal consequence of a timeout gap: after votes for a height are lost,
//! the next leader proposes a recovery block carrying the older HighQC plus a TC, so every node
//! stores that block (and the dummy fillers beneath it) while its HighPC stays back at the last
//! certified height.
//!
//! When such a node then suffers three consecutive leader failures, `request_catch_up_sync`
//! rewinds the pacemaker view *down* to the HighPC height — i.e. **below blocks it has already
//! stored**. Catch-up re-delivers those blocks as ordinary `Proposal` messages, but the receiver
//! dedups them (`record_exists` -> "already processed") and returns without advancing the view.
//! The view can therefore never climb back to the stored tip: every higher proposal is buffered as
//! "future", every catch-up round re-rewinds to the same height, and the node is wedged forever
//! while the rest of the committee runs on.
//!
//! Reproduction: let the target fully catch up, induce a single timeout gap right at its tip so it
//! stores a recovery block above its HighPC, freeze it there (partition) until it suffers the
//! rewind, then restore connectivity. Recovery is asserted on the target's HighPC climbing back to
//! the network tip — not merely on the batch committing, since those transactions finalise before
//! the freeze and a wedged node would still report them committed while its HighPC stays pinned at
//! the rewind height forever. Without the fix the target wedges exactly so; with it, it re-joins.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use tari_consensus::messages::HotstuffMessage;
use tari_consensus_types::{Decision, HighPc, HighestSeenBlock};
use tari_ootle_common_types::{Epoch, NodeHeight};
use tari_ootle_storage::{StateStore, consensus_models::BookkeepingModel};

use crate::support::{Test, TestAddress, TestVnDestination, logging::setup_logger};

#[expect(clippy::too_many_lines)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catch_up_rewind_below_leaf_recovers() {
    setup_logger();

    let votes_dropped = Arc::new(AtomicUsize::new(0));
    // First height of the three-height vote-drop window that manufactures one timeout gap. 0 until
    // the loop arms it (once the target is fully caught up).
    let gap_at = Arc::new(AtomicU64::new(0));
    // Set by the filter when it has delivered the gap's recovery block to the target and partitioned
    // it; the target now holds a stored tip above its HighPC.
    let frozen = Arc::new(AtomicBool::new(false));
    // While true, every message to the target is dropped, forcing the leader-failure → catch-up →
    // view-rewind cascade.
    let partition = Arc::new(AtomicBool::new(false));

    let target = TestAddress::new("2");

    let votes_dropped_f = votes_dropped.clone();
    let gap_at_f = gap_at.clone();
    let frozen_f = frozen.clone();
    let partition_f = partition.clone();
    let target_f = target.clone();

    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(240))
        .modify_consensus_constants(|c| {
            // Tight pacemaker so the three leader failures (and the rewind) happen quickly.
            c.pacemaker_block_time = Duration::from_secs(2);
            // The whole point is recovery, so never suspend the isolated node for missed proposals.
            c.missed_proposal_suspend_threshold = 50;
        })
        .with_message_filter(Box::new(move |_from, to, msg| {
            let gap = gap_at_f.load(Ordering::SeqCst);

            // (1) Manufacture a single timeout gap at the armed height (drop a small window of
            // consecutive heights so the gap reliably forms).
            if gap != 0 &&
                let HotstuffMessage::Vote(v) = msg
            {
                let h = v.vote.block_height.as_u64();
                if h >= gap && h <= gap + 6 {
                    votes_dropped_f.fetch_add(1, Ordering::SeqCst);
                    return false;
                }
            }

            if *to != target_f {
                return true;
            }

            // Already partitioned: drop everything to the target.
            if partition_f.load(Ordering::SeqCst) {
                return false;
            }

            // (2) Freeze the target on the gap's recovery block: a proposal carrying a TC whose
            // justify sits more than one height below it (a dummy gap). The target is caught up, so
            // it applies the block immediately — leaving a stored tip above its HighPC — and we
            // partition it there.
            if gap != 0 &&
                !frozen_f.load(Ordering::SeqCst) &&
                let HotstuffMessage::Proposal(p) = msg &&
                p.block.timeout_certificate().is_some() &&
                p.block.justify().height() + NodeHeight(1) < p.block.height()
            {
                frozen_f.store(true, Ordering::SeqCst);
                partition_f.store(true, Ordering::SeqCst);
                return true;
            }

            true
        }))
        .add_committee(0, vec!["1", "2", "3", "4"])
        .start()
        .await;

    // A batch of transactions the committee must finalise. The online quorum (3 of 4) will commit
    // them while the target is isolated; the target must catch up and commit them too.
    let mut tx_ids = Vec::new();
    for _ in 0..5 {
        let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
        tx_ids.push(*tx.id());
    }

    test.start_epoch(Epoch(1)).await;

    enum Phase {
        Warmup,
        Armed,
        Frozen { froze_at: NodeHeight },
        Lifted,
    }
    let mut phase = Phase::Warmup;
    // Highest height committed anywhere in the committee. `on_block_committed` fires per-node, so a
    // single event's height can dip (e.g. the target committing an early block as it recovers); we
    // track the running max for the network's true progress.
    let mut network_max = NodeHeight::zero();
    loop {
        let (_, _, _, committed_height) = test.on_block_committed().await;
        network_max = network_max.max(committed_height);

        let seen = target_highest_seen(&test, &target);
        let high_pc = target_high_pc(&test, &target);

        match &phase {
            // Wait for the chain to get going and the target to be fully caught up, then arm a gap a
            // couple of heights ahead — right at the target's tip, where it can apply the recovery
            // block immediately.
            Phase::Warmup => {
                if network_max >= NodeHeight(8) && high_pc + NodeHeight(3) >= network_max {
                    let arm = network_max + NodeHeight(2);
                    gap_at.store(arm.as_u64(), Ordering::SeqCst);
                    log::info!("🎯 Arming timeout gap at height {arm} (target caught up at HighPC {high_pc})");
                    phase = Phase::Armed;
                }
            },
            // The filter froze & partitioned the target on the recovery block; confirm it holds a
            // stored tip above its HighPC.
            Phase::Armed => {
                if frozen.load(Ordering::SeqCst) && seen >= high_pc + NodeHeight(2) {
                    log::info!(
                        "❄️ Froze {target} on gap tip: HighestSeen {seen} >= HighPC {high_pc} + 2 (network height \
                         {network_max})"
                    );
                    phase = Phase::Frozen { froze_at: network_max };
                }
            },
            // Give the partitioned target time for three leader failures + the catch-up rewind, then
            // restore connectivity and require it to re-join. The window must be long enough for the
            // target to hit its *third* consecutive leader timeout, which is what triggers the
            // `request_catch_up_sync` view-rewind.
            Phase::Frozen { froze_at } => {
                if network_max >= *froze_at + NodeHeight(18) {
                    assert!(
                        seen > high_pc,
                        "test premise: frozen target should hold a stored tip ({seen}) above HighPC ({high_pc})"
                    );
                    log::info!(
                        "🔌 Restoring {target} at network height {network_max} (HighestSeen {seen}, HighPC {high_pc})"
                    );
                    partition.store(false, Ordering::SeqCst);
                    phase = Phase::Lifted;
                }
            },
            // Success requires *genuine* recovery: the target's HighPC must climb back up to the
            // network tip. (Checking only that the batch is committed is not enough — those
            // transactions are finalised early, before the freeze, so a wedged target would still
            // report them committed.) A node stuck in the catch-up rewind keeps its HighPC pinned at
            // the rewind height forever, so this is exactly what the bug breaks.
            Phase::Lifted => {
                let committed = tx_ids
                    .iter()
                    .all(|id| test.get_validator(&target).has_committed_substates(id));
                if committed && high_pc + NodeHeight(4) >= network_max {
                    log::info!("✅ {target} recovered: HighPC {high_pc} caught up to network height {network_max}");
                    break;
                }
            },
        }

        if network_max > NodeHeight(70) {
            let reached = match phase {
                Phase::Warmup => "Warmup (never armed)",
                Phase::Armed => "Armed (gap never froze the target)",
                Phase::Frozen { .. } => "Frozen (never lifted)",
                Phase::Lifted => "Lifted (restored but stayed wedged — HighPC pinned at the rewind height)",
            };
            panic!(
                "{target} failed to re-join consensus after connectivity was restored (network height {network_max}, \
                 target HighestSeen {seen}, target HighPC {high_pc}, phase {reached}, votes_dropped={}). This is the \
                 catch-up view-rewind deadlock.",
                votes_dropped.load(Ordering::SeqCst),
            );
        }
    }

    assert!(
        votes_dropped.load(Ordering::SeqCst) > 0,
        "test premise: votes at the gap height must have been dropped"
    );

    // Everyone — including the recovered target — finalises the batch.
    test.wait_for_pool_count(TestVnDestination::All, 0).await;
    test.stop();
    test.assert_clean_shutdown().await;
}

fn target_high_pc(test: &Test, target: &TestAddress) -> NodeHeight {
    test.get_validator(target)
        .state_store()
        .with_read_tx(|tx| HighPc::get(tx, Epoch(1)))
        .map(|h| h.height())
        .expect("Failed to retrieve HighPc from state store")
}

fn target_highest_seen(test: &Test, target: &TestAddress) -> NodeHeight {
    test.get_validator(target)
        .state_store()
        .with_read_tx(|tx| HighestSeenBlock::get(tx, Epoch(1)))
        .map(|h| h.height())
        .expect("Failed to retrieve HighestSeenBlock from state store")
}
