//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_ootle_common_types::ShardGroup;
use tari_ootle_storage::consensus_models::{TransactionPoolRecord, TransactionPoolStage};

/// How far a leader has drained cross-shard transactions ahead of an end of epoch that changes the number of
/// committees.
///
/// Such a boundary re-divides the shards between the committees, so validators move to new shard groups, state-sync
/// into them and drop their transaction pools. A cross-shard transaction still between `LocalPrepared` and
/// finalization at that point keeps its substate locks with no pool record left to finalize it. The leader therefore
/// drains these transactions before it proposes `EndEpoch`:
///
/// 1. For the drain period after the next epoch is known, it stops preparing new cross-shard transactions and lets
///    those already prepared finish.
/// 2. After it, it aborts every transaction still waiting in `LocalPrepared`, which is safe because the local shard
///    group has not sent its LocalAccept.
/// 3. `EndEpoch` waits until no transaction is in `LocalPrepared` or `LocalAccepted`. A transaction in `LocalAccepted`
///    cannot be aborted, so after a further drain period `EndEpoch` stops waiting for it. Only a foreign shard group
///    that has stopped delivering keeps one there that long.
///
/// These are proposing rules. The only one a replica checks is that an abort from `LocalPrepared` is permitted, see
/// [`abort_local_prepared_for_epoch_end`](crate::hotstuff::abort_local_prepared_for_epoch_end).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EpochEndDrain {
    /// The next epoch keeps the number of committees, or is not known yet.
    #[default]
    Inactive,
    /// New cross-shard transactions are held back while those already prepared finish.
    HoldingBack,
    /// Transactions waiting in `LocalPrepared` are aborted.
    Aborting,
    /// As `Aborting`, and `EndEpoch` no longer waits for transactions in `LocalAccepted`.
    Overdue,
}

impl EpochEndDrain {
    /// The drain phase `elapsed` after the next epoch, which changes the number of committees, became known.
    pub fn after(elapsed: Duration, drain_period: Duration) -> Self {
        if elapsed < drain_period {
            Self::HoldingBack
        } else if elapsed < drain_period.saturating_mul(2) {
            Self::Aborting
        } else {
            Self::Overdue
        }
    }

    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Inactive)
    }

    pub fn aborts_waiting_prepared(&self) -> bool {
        matches!(self, Self::Aborting | Self::Overdue)
    }

    /// True if `rec` is a cross-shard transaction that has not been prepared by the local shard group, and this
    /// drain holds it back.
    pub fn holds_back(&self, rec: &TransactionPoolRecord, local_shard_group: ShardGroup) -> bool {
        self.is_active() &&
            rec.current_stage().is_new() &&
            rec.current_decision().is_commit() &&
            rec.evidence().num_shard_groups() > 1 &&
            rec.evidence().has_inputs(local_shard_group)
    }

    /// True if `EndEpoch` may not be proposed while a pooled transaction is in `stage`.
    pub fn holds_epoch_end_for(&self, stage: TransactionPoolStage) -> bool {
        match self {
            Self::Inactive => false,
            Self::HoldingBack | Self::Aborting => stage.is_local_prepared() || stage.is_local_accepted(),
            Self::Overdue => stage.is_local_prepared(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_moves_through_the_phases_with_the_drain_period() {
        let period = Duration::from_secs(10);
        assert_eq!(EpochEndDrain::after(Duration::ZERO, period), EpochEndDrain::HoldingBack);
        assert_eq!(
            EpochEndDrain::after(Duration::from_secs(10), period),
            EpochEndDrain::Aborting
        );
        assert_eq!(
            EpochEndDrain::after(Duration::from_secs(20), period),
            EpochEndDrain::Overdue
        );
    }

    #[test]
    fn epoch_end_stops_waiting_for_local_accepted_once_overdue() {
        for drain in [EpochEndDrain::HoldingBack, EpochEndDrain::Aborting] {
            assert!(drain.holds_epoch_end_for(TransactionPoolStage::LocalPrepared));
            assert!(drain.holds_epoch_end_for(TransactionPoolStage::LocalAccepted));
            assert!(!drain.holds_epoch_end_for(TransactionPoolStage::New));
            assert!(!drain.holds_epoch_end_for(TransactionPoolStage::AllAccepted));
        }
        assert!(EpochEndDrain::Overdue.holds_epoch_end_for(TransactionPoolStage::LocalPrepared));
        assert!(!EpochEndDrain::Overdue.holds_epoch_end_for(TransactionPoolStage::LocalAccepted));
        assert!(!EpochEndDrain::Inactive.holds_epoch_end_for(TransactionPoolStage::LocalPrepared));
    }
}
