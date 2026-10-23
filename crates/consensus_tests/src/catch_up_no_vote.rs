//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A replica does not vote on the blocks it imports through catch-up.
//!
//! Catch-up imports blocks the committee has already certified, so a vote on one can never help form a certificate.
//! It can conflict with a vote the replica cast before it lost its voting records, e.g. one restored from a backup
//! snapshot. The test withholds three consecutive proposals from one replica so that it imports them through
//! catch-up, then requires that it votes on none of them and does vote on a later live proposal.

use std::{
    collections::HashSet,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tari_consensus::messages::HotstuffMessage;
use tari_consensus_types::{BlockId, Decision};
use tari_ootle_common_types::{Epoch, NodeHeight};

use crate::support::{MessageFilter, Test, TestAddress, logging::setup_logger};

/// Consecutive proposals withheld from the target.
const DROPPED_PROPOSALS: u64 = 3;

/// Records the target's votes and the blocks it receives through catch-up, and withholds from it the proposals in
/// the window starting at `drop_from` once that is armed.
fn observe_target(
    target: TestAddress,
    drop_from: Arc<AtomicU64>,
    withheld: Arc<Mutex<HashSet<BlockId>>>,
    caught_up: Arc<Mutex<HashSet<BlockId>>>,
    votes: Arc<Mutex<Vec<(BlockId, NodeHeight)>>>,
) -> MessageFilter {
    Box::new(move |from, to, msg| {
        if *from == target {
            if let HotstuffMessage::Vote(vote) = msg {
                votes.lock().unwrap().push((vote.vote.block_id, vote.vote.block_height));
            }
            return true;
        }
        if *to != target {
            return true;
        }
        match msg {
            HotstuffMessage::CatchUpSyncResponse(response) => {
                caught_up.lock().unwrap().insert(*response.block.id());
                true
            },
            HotstuffMessage::Proposal(proposal) => {
                let start = drop_from.load(Ordering::SeqCst);
                let height = proposal.block.height().as_u64();
                if start != 0 && (start..start + DROPPED_PROPOSALS).contains(&height) {
                    withheld.lock().unwrap().insert(*proposal.block.id());
                    return false;
                }
                true
            },
            _ => true,
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_replica_does_not_vote_on_blocks_it_catches_up_on() {
    setup_logger();

    let target = TestAddress::new("4");
    // First height of the withheld window; 0 until armed.
    let drop_from = Arc::new(AtomicU64::new(0));
    let withheld = Arc::new(Mutex::new(HashSet::<BlockId>::new()));
    let caught_up = Arc::new(Mutex::new(HashSet::<BlockId>::new()));
    let votes = Arc::new(Mutex::new(Vec::<(BlockId, NodeHeight)>::new()));

    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(120))
        .modify_consensus_constants(|c| {
            c.pacemaker_block_time = Duration::from_secs(2);
            c.missed_proposal_suspend_threshold = 50;
        })
        .with_message_filter(observe_target(
            target.clone(),
            drop_from.clone(),
            withheld.clone(),
            caught_up.clone(),
            votes.clone(),
        ))
        .add_committee(0, vec!["1", "2", "3", "4"])
        .start()
        .await;

    for _ in 0..3 {
        test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
    }
    test.start_epoch(Epoch(1)).await;

    let mut network_max = NodeHeight::zero();
    loop {
        let (_, _, _, committed_height) = test.on_block_committed().await;
        network_max = network_max.max(committed_height);
        let target_leaf = test.get_validator(&target).get_leaf_block().height();

        let start = drop_from.load(Ordering::SeqCst);
        if start == 0 {
            if network_max >= NodeHeight(5) && target_leaf + NodeHeight(3) >= network_max {
                let start = network_max + NodeHeight(3);
                log::info!("🎯 Withholding proposals {start}..+{DROPPED_PROPOSALS} from {target}");
                drop_from.store(start.as_u64(), Ordering::SeqCst);
            }
        } else {
            let after_window = NodeHeight(start + DROPPED_PROPOSALS);
            let voted_after_window = votes.lock().unwrap().iter().any(|(_, h)| *h >= after_window);
            if target_leaf >= after_window && voted_after_window {
                break;
            }
        }

        assert!(
            network_max <= NodeHeight(60),
            "{target} never voted past the withheld window (network height {network_max}, target leaf {target_leaf})"
        );
    }

    let withheld = withheld.lock().unwrap().clone();
    let caught_up = caught_up.lock().unwrap().clone();
    assert!(
        !withheld.is_empty(),
        "test premise: proposals to the target must have been withheld"
    );
    assert!(
        withheld.iter().any(|id| caught_up.contains(id)),
        "test premise: the target must have imported a withheld block through catch-up"
    );
    let votes_on_withheld = votes
        .lock()
        .unwrap()
        .iter()
        .filter(|(id, _)| withheld.contains(id))
        .copied()
        .collect::<Vec<_>>();
    assert!(
        votes_on_withheld.is_empty(),
        "{target} voted on blocks it imported through catch-up: {votes_on_withheld:?}"
    );

    test.stop();
    test.assert_clean_shutdown().await;
}
