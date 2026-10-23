//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A replica does not vote on the blocks it imports through catch-up, but does receive its catch-up server's leaf as
//! a proposal.
//!
//! Catch-up imports blocks the committee has already certified, so a vote on one can never help form a certificate.
//! It can conflict with a vote the replica cast before it lost its voting records, e.g. one restored from a backup
//! snapshot. The server's leaf has no certificate yet, so it is sent as a proposal the replica may vote on. The test
//! withholds three consecutive proposals from one replica so that it imports them through catch-up, then requires
//! that it votes on none of them, that its server relays a leaf as a proposal, and that it votes on a later live
//! proposal.

use std::{
    collections::HashSet,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use ootle_byte_type::ToByteType;
use tari_consensus::messages::HotstuffMessage;
use tari_consensus_types::{BlockId, Decision};
use tari_ootle_common_types::{Epoch, NodeHeight};

use crate::support::{MessageFilter, Test, TestAddress, helpers, logging::setup_logger};

/// Consecutive proposals withheld from the target.
const DROPPED_PROPOSALS: u64 = 3;

#[derive(Default)]
struct Observed {
    /// Every proposal delivered to the target, by sender.
    proposal_deliveries: HashSet<(TestAddress, BlockId)>,
    /// Proposals their proposers broadcast to the target and the filter withheld.
    withheld: HashSet<BlockId>,
    /// Blocks the target received in catch-up responses.
    caught_up: HashSet<BlockId>,
    /// Proposals the target received other than as their proposer's broadcast, i.e. a catch-up server's leaf: from a
    /// validator other than the proposer, or a second time from the proposer.
    relayed: HashSet<BlockId>,
    /// The target's votes.
    votes: Vec<(BlockId, NodeHeight)>,
}

/// Records what the target sends and receives, and withholds from it the proposals broadcast in the window starting
/// at `drop_from` once that is armed. A relayed proposal is never withheld.
fn observe_target(target: TestAddress, drop_from: Arc<AtomicU64>, observed: Arc<Mutex<Observed>>) -> MessageFilter {
    Box::new(move |from, to, msg| {
        if *from == target {
            if let HotstuffMessage::Vote(vote) = msg {
                observed
                    .lock()
                    .unwrap()
                    .votes
                    .push((vote.vote.block_id, vote.vote.block_height));
            }
            return true;
        }
        if *to != target {
            return true;
        }
        match msg {
            HotstuffMessage::CatchUpSyncResponse(response) => {
                observed.lock().unwrap().caught_up.insert(*response.block.id());
                true
            },
            HotstuffMessage::Proposal(proposal) => {
                let block_id = *proposal.block.id();
                let mut observed = observed.lock().unwrap();
                let is_first_delivery = observed.proposal_deliveries.insert((from.clone(), block_id));
                let (_, sender_pk) = helpers::derive_keypair_from_address(from);
                if !is_first_delivery || *proposal.block.proposed_by() != sender_pk.to_byte_type() {
                    observed.relayed.insert(block_id);
                    return true;
                }
                let start = drop_from.load(Ordering::SeqCst);
                let height = proposal.block.height().as_u64();
                if start != 0 && (start..start + DROPPED_PROPOSALS).contains(&height) {
                    observed.withheld.insert(block_id);
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
    let observed = Arc::new(Mutex::new(Observed::default()));

    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(120))
        .modify_consensus_constants(|c| {
            c.pacemaker_block_time = Duration::from_secs(2);
            c.missed_proposal_suspend_threshold = 50;
        })
        .with_message_filter(observe_target(target.clone(), drop_from.clone(), observed.clone()))
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
            let voted_after_window = observed.lock().unwrap().votes.iter().any(|(_, h)| *h >= after_window);
            if target_leaf >= after_window && voted_after_window {
                break;
            }
        }

        assert!(
            network_max <= NodeHeight(60),
            "{target} never voted past the withheld window (network height {network_max}, target leaf {target_leaf})"
        );
    }

    {
        let observed = observed.lock().unwrap();
        assert!(
            !observed.withheld.is_empty(),
            "test premise: proposals to the target must have been withheld"
        );
        assert!(
            observed.withheld.iter().any(|id| observed.caught_up.contains(id)),
            "test premise: the target must have imported a withheld block through catch-up"
        );
        let votes_on_withheld = observed
            .votes
            .iter()
            .filter(|(id, _)| observed.withheld.contains(id))
            .collect::<Vec<_>>();
        assert!(
            votes_on_withheld.is_empty(),
            "{target} voted on blocks it imported through catch-up: {votes_on_withheld:?}"
        );
        assert!(
            !observed.relayed.is_empty(),
            "{target} caught up without its catch-up server sending its leaf as a proposal"
        );
    }

    test.stop();
    test.assert_clean_shutdown().await;
}
