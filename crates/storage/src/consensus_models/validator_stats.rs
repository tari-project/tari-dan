//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_ootle_common_types::{Epoch, NodeHeight};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{StateStoreReadTransaction, StorageError};

/// The thresholds that turn a validator's liveness counters into a [`LivenessState`].
///
/// Carried with every update so that the transition table is a pure function of the stored counters
/// and these values: the store applies it without knowing anything about consensus configuration.
/// CONSENSUS RULE: must be uniform network-wide — leader selection reads the state these produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LivenessThresholds {
    /// Missed proposals that suspend a validator.
    pub suspend_after_missed: u64,
    /// Votes a suspended validator must land in committed blocks before its first probation slot.
    pub probation_base_votes: u64,
    /// Committed blocks after which a suspended validator gets its first probation slot even if none
    /// of its votes have reached a certificate.
    pub probation_base_blocks: u64,
    /// Cap on the exponent in `probation_base_votes × 2^probation_failures` and in
    /// `probation_base_blocks × 2^probation_failures`.
    pub probation_max_backoff_exp: u32,
}

/// Where a validator sits in the liveness cycle for an epoch. Derived from [`LivenessCounters`],
/// never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivenessState {
    /// Proposing normally, or not yet missed enough proposals to be suspended.
    Normal,
    /// Missed too many proposals and has not since participated enough to earn a slot back.
    Suspended,
    /// Has earned one slot back. Proposing a block that commits returns it to `Normal`; missing the
    /// slot returns it to `Suspended` with the next wait doubled.
    Probation,
}

impl LivenessState {
    /// Whether leader selection skips this validator's slot.
    pub fn is_skipped(&self) -> bool {
        matches!(self, LivenessState::Suspended)
    }
}

/// The counters that decide a validator's [`LivenessState`].
///
/// Split out from [`ValidatorConsensusStats`] because these are what the liveness log records per
/// committed height: `participation_shares` changes on every commit for every signer and is not part
/// of the state machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Encode, Decode, CborLen)]
pub struct LivenessCounters {
    #[n(0)]
    pub missed_proposals: u64,
    #[n(1)]
    pub probation_failures: u32,
    #[n(2)]
    pub votes_since_suspended: u64,
    /// The height of the block whose commit last suspended this validator.
    #[n(3)]
    pub suspended_at_height: NodeHeight,
}

impl LivenessCounters {
    /// The state as of the given committed height. The height is part of the question because a
    /// suspension expires after a number of committed blocks, so two nodes reading at different
    /// heights would otherwise disagree.
    pub fn liveness_state(&self, thresholds: &LivenessThresholds, as_of: NodeHeight) -> LivenessState {
        if self.missed_proposals < thresholds.suspend_after_missed {
            return LivenessState::Normal;
        }

        if self.votes_since_suspended >= self.votes_required_for_probation(thresholds) {
            return LivenessState::Probation;
        }

        if as_of.saturating_sub(self.suspended_at_height).as_u64() >= self.blocks_required_for_probation(thresholds) {
            return LivenessState::Probation;
        }

        LivenessState::Suspended
    }

    pub fn is_skipped(&self, thresholds: &LivenessThresholds, as_of: NodeHeight) -> bool {
        self.liveness_state(thresholds, as_of).is_skipped()
    }

    /// Votes this validator must land in committed blocks before its next probation slot.
    pub fn votes_required_for_probation(&self, thresholds: &LivenessThresholds) -> u64 {
        thresholds.probation_base_votes.saturating_mul(self.backoff(thresholds))
    }

    /// Committed blocks after which this validator gets its next probation slot regardless of its
    /// votes, which is what guarantees it gets one. A vote only counts towards the slot once it is
    /// in the certificate of a committed block, and a leader stops collecting at a quorum, so a
    /// validator that is merely slower than its committee can have every vote of its arrive too late
    /// to count.
    pub fn blocks_required_for_probation(&self, thresholds: &LivenessThresholds) -> u64 {
        thresholds
            .probation_base_blocks
            .saturating_mul(self.backoff(thresholds))
    }

    /// Doubles per failed probation so that a validator that is given slots and does not use them
    /// costs the network a geometrically decreasing number of them, without ever being excluded for
    /// good.
    fn backoff(&self, thresholds: &LivenessThresholds) -> u64 {
        1u64 << self.probation_failures.min(thresholds.probation_max_backoff_exp)
    }
}

/// A change to one validator's epoch stats, applied by [`ValidatorConsensusStats::apply`].
///
/// The caller states what happened - a missed proposal, a committed proposal, a vote that made it
/// into a committed block - and the transition table decides which counters move.
#[derive(Debug, Clone, Copy)]
pub struct ValidatorStatsUpdate<'a> {
    public_key: &'a RistrettoPublicKeyBytes,
    thresholds: LivenessThresholds,
    event: Option<LivenessEvent>,
    is_vote: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LivenessEvent {
    MissedProposal,
    ProposalCommitted,
}

impl<'a> ValidatorStatsUpdate<'a> {
    pub fn new(public_key: &'a RistrettoPublicKeyBytes, thresholds: LivenessThresholds) -> Self {
        Self {
            public_key,
            thresholds,
            event: None,
            is_vote: false,
        }
    }

    pub fn public_key(&self) -> &RistrettoPublicKeyBytes {
        self.public_key
    }

    pub fn thresholds(&self) -> &LivenessThresholds {
        &self.thresholds
    }

    /// The validator was the leader of a view that produced a dummy block.
    pub fn add_missed_proposal(mut self) -> Self {
        self.event = Some(LivenessEvent::MissedProposal);
        self
    }

    /// The validator proposed a block that has now committed.
    pub fn reset_missed_proposals(mut self) -> Self {
        self.event = Some(LivenessEvent::ProposalCommitted);
        self
    }

    /// The validator's vote is in the certificate of a block that has now committed.
    pub fn record_vote(mut self) -> Self {
        self.is_vote = true;
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Encode, Decode, CborLen)]
pub struct ValidatorConsensusStats {
    #[n(0)]
    pub missed_proposals: u64,
    #[n(1)]
    pub participation_shares: u64,
    #[n(2)]
    pub probation_failures: u32,
    #[n(3)]
    pub votes_since_suspended: u64,
    #[n(4)]
    pub suspended_at_height: NodeHeight,
}

impl ValidatorConsensusStats {
    pub fn get_by_public_key<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        epoch: Epoch,
        public_key: &RistrettoPublicKeyBytes,
    ) -> Result<Self, StorageError> {
        tx.validator_epoch_stats_get(epoch, public_key)
    }

    /// The liveness state of `public_key` as of the given committed height, for the leader selection
    /// of a view anchored on that height. `None` means no counter has moved for this validator by
    /// that height, which is [`LivenessState::Normal`].
    pub fn liveness_counters_as_of<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        epoch: Epoch,
        public_key: &RistrettoPublicKeyBytes,
        as_of: NodeHeight,
    ) -> Result<LivenessCounters, StorageError> {
        Ok(tx
            .validator_liveness_counters_as_of(epoch, public_key, as_of)?
            .unwrap_or_default())
    }

    pub fn counters(&self) -> LivenessCounters {
        LivenessCounters {
            missed_proposals: self.missed_proposals,
            probation_failures: self.probation_failures,
            votes_since_suspended: self.votes_since_suspended,
            suspended_at_height: self.suspended_at_height,
        }
    }

    pub fn liveness_state(&self, thresholds: &LivenessThresholds, as_of: NodeHeight) -> LivenessState {
        self.counters().liveness_state(thresholds, as_of)
    }

    /// Applies an update caused by the commit of the block at `committed_height` and returns whether
    /// it moved any counter that decides the liveness state. Only those changes need a liveness log
    /// entry; participation shares move on every commit.
    pub fn apply(&mut self, update: &ValidatorStatsUpdate<'_>, committed_height: NodeHeight) -> bool {
        let before = self.counters();
        let state = before.liveness_state(&update.thresholds, committed_height);

        if update.is_vote {
            self.participation_shares += 1;
            // Participation earns a probation slot back. It is counted only while suspended: once
            // the slot is earned the validator has to take it, and a validator in good standing has
            // nothing to earn.
            if state == LivenessState::Suspended {
                self.votes_since_suspended += 1;
            }
        }

        match update.event {
            Some(LivenessEvent::MissedProposal) => {
                self.missed_proposals += 1;
                if state == LivenessState::Probation {
                    // The slot it was given to prove it is back is the one it just missed.
                    self.probation_failures += 1;
                    self.votes_since_suspended = 0;
                }
                // The wait for the next probation slot runs from the miss that started it.
                if state != LivenessState::Suspended && self.missed_proposals >= update.thresholds.suspend_after_missed
                {
                    self.suspended_at_height = committed_height;
                }
            },
            Some(LivenessEvent::ProposalCommitted) => {
                self.missed_proposals = 0;
                self.probation_failures = 0;
                self.votes_since_suspended = 0;
                self.suspended_at_height = NodeHeight::zero();
            },
            None => {},
        }

        self.counters() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cooldown far longer than any test chain, so that a test says what earns the probation slot
    /// rather than leaving it to whichever path comes first.
    const THRESHOLDS: LivenessThresholds = LivenessThresholds {
        suspend_after_missed: 5,
        probation_base_votes: 5,
        probation_base_blocks: 1_000,
        probation_max_backoff_exp: 6,
    };

    const PK: RistrettoPublicKeyBytes = RistrettoPublicKeyBytes::zero();

    /// One validator's counters over a chain of committed blocks, one block per event.
    struct Chain {
        stats: ValidatorConsensusStats,
        height: NodeHeight,
        thresholds: LivenessThresholds,
    }

    impl Chain {
        fn new() -> Self {
            Self::with_thresholds(THRESHOLDS)
        }

        fn with_thresholds(thresholds: LivenessThresholds) -> Self {
            Self {
                stats: ValidatorConsensusStats::default(),
                height: NodeHeight::zero(),
                thresholds,
            }
        }

        fn apply(&mut self, update: ValidatorStatsUpdate<'_>) -> bool {
            self.height += NodeHeight(1);
            self.stats.apply(&update, self.height)
        }

        fn update(&self) -> ValidatorStatsUpdate<'static> {
            ValidatorStatsUpdate::new(&PK, self.thresholds)
        }

        fn miss(&mut self) -> bool {
            self.apply(self.update().add_missed_proposal())
        }

        fn vote(&mut self) -> bool {
            self.apply(self.update().record_vote())
        }

        fn propose(&mut self) -> bool {
            self.apply(self.update().reset_missed_proposals())
        }

        /// Blocks committed by others, which move nothing of this validator's but do move the height
        /// its state is read at.
        fn idle(&mut self, blocks: u64) {
            self.height += NodeHeight(blocks);
        }

        fn state(&self) -> LivenessState {
            self.stats.liveness_state(&self.thresholds, self.height)
        }

        fn counters(&self) -> LivenessCounters {
            self.stats.counters()
        }
    }

    #[test]
    fn misses_below_the_threshold_stay_normal() {
        let mut chain = Chain::new();
        for _ in 0..4 {
            chain.miss();
            assert_eq!(chain.state(), LivenessState::Normal);
        }
        chain.miss();
        assert_eq!(chain.state(), LivenessState::Suspended);
    }

    #[test]
    fn a_suspended_validator_that_neither_votes_nor_waits_stays_suspended() {
        let mut chain = Chain::new();
        for _ in 0..10 {
            chain.miss();
        }
        assert_eq!(chain.state(), LivenessState::Suspended);
    }

    #[test]
    fn votes_earn_a_probation_slot() {
        let mut chain = Chain::new();
        for _ in 0..5 {
            chain.miss();
        }
        for _ in 0..4 {
            chain.vote();
            assert_eq!(chain.state(), LivenessState::Suspended);
        }
        chain.vote();
        assert_eq!(chain.state(), LivenessState::Probation);
    }

    /// A vote only counts once it is in the certificate of a committed block, and a leader stops
    /// collecting at a quorum. A validator whose votes never make one still gets a slot back.
    #[test]
    fn waiting_earns_a_probation_slot_without_a_single_vote() {
        let mut chain = Chain::with_thresholds(LivenessThresholds {
            probation_base_blocks: 20,
            ..THRESHOLDS
        });
        for _ in 0..5 {
            chain.miss();
        }
        assert_eq!(chain.state(), LivenessState::Suspended);

        chain.idle(19);
        assert_eq!(chain.state(), LivenessState::Suspended);
        chain.idle(1);
        assert_eq!(chain.state(), LivenessState::Probation);
    }

    #[test]
    fn a_committed_proposal_clears_everything() {
        let mut chain = Chain::new();
        for _ in 0..5 {
            chain.miss();
        }
        for _ in 0..5 {
            chain.vote();
        }
        assert_eq!(chain.state(), LivenessState::Probation);

        chain.propose();
        assert_eq!(chain.state(), LivenessState::Normal);
        assert_eq!(chain.counters(), LivenessCounters::default());
    }

    #[test]
    fn a_missed_probation_slot_doubles_both_waits() {
        let mut chain = Chain::with_thresholds(LivenessThresholds {
            probation_base_blocks: 20,
            ..THRESHOLDS
        });
        for _ in 0..5 {
            chain.miss();
        }
        for _ in 0..5 {
            chain.vote();
        }
        assert_eq!(chain.state(), LivenessState::Probation);

        // The probation slot produces a dummy block, which is charged to it.
        chain.miss();
        assert_eq!(chain.state(), LivenessState::Suspended);
        assert_eq!(chain.counters().probation_failures, 1);
        assert_eq!(chain.counters().votes_since_suspended, 0);
        assert_eq!(chain.counters().votes_required_for_probation(&chain.thresholds), 10);
        assert_eq!(chain.counters().blocks_required_for_probation(&chain.thresholds), 40);

        for _ in 0..9 {
            chain.vote();
        }
        assert_eq!(chain.state(), LivenessState::Suspended);
        chain.vote();
        assert_eq!(chain.state(), LivenessState::Probation);
    }

    /// The wait runs from the miss that suspended the validator, not from the epoch.
    #[test]
    fn the_wait_runs_from_the_suspension() {
        let mut chain = Chain::with_thresholds(LivenessThresholds {
            probation_base_blocks: 20,
            ..THRESHOLDS
        });
        chain.idle(100);
        for _ in 0..5 {
            chain.miss();
        }
        let suspended_at = chain.counters().suspended_at_height;
        assert_eq!(suspended_at, chain.height);

        chain.idle(19);
        assert_eq!(chain.state(), LivenessState::Suspended);
        chain.idle(1);
        assert_eq!(chain.state(), LivenessState::Probation);
    }

    #[test]
    fn the_backoff_exponent_is_capped() {
        let counters = LivenessCounters {
            missed_proposals: 5,
            probation_failures: 100,
            votes_since_suspended: 0,
            suspended_at_height: NodeHeight::zero(),
        };
        assert_eq!(counters.votes_required_for_probation(&THRESHOLDS), 5 * 64);
        assert_eq!(counters.blocks_required_for_probation(&THRESHOLDS), 1_000 * 64);
    }

    #[test]
    fn votes_are_not_counted_outside_suspension() {
        let mut chain = Chain::new();
        chain.vote();
        assert_eq!(chain.stats.participation_shares, 1);
        assert_eq!(chain.counters().votes_since_suspended, 0);

        // ...nor once the probation slot is earned: it has to be taken.
        for _ in 0..5 {
            chain.miss();
        }
        for _ in 0..5 {
            chain.vote();
        }
        assert_eq!(chain.state(), LivenessState::Probation);
        chain.vote();
        assert_eq!(chain.counters().votes_since_suspended, 5);
    }

    #[test]
    fn only_liveness_counters_report_a_change() {
        let mut chain = Chain::new();
        assert!(!chain.vote());
        assert!(chain.miss());
        // A reset of counters that are already zero changes nothing.
        let mut chain = Chain::new();
        assert!(!chain.propose());
    }
}
