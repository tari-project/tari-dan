//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use tari_ootle_common_types::{Epoch, ShardGroup};

/// The epoch the validators execute transactions in, as this indexer has observed it from their
/// quorum-verified committed tips.
///
/// The epoch manager moves to an epoch as soon as the base layer scan reaches its boundary, but a
/// committee only moves at its end-of-epoch block, so for a while after every boundary the
/// committees still execute in the previous epoch. Anything that must agree with how a transaction
/// will execute - a dry run, or a client deciding whether the network is past an epoch - reads this
/// instead of the epoch manager.
///
/// It is the lowest epoch across the shard groups of the current sync plan: a transaction may land
/// in any of them, so the network is only in an epoch once every group is.
#[derive(Debug, Clone, Default)]
pub struct ConsensusEpoch {
    inner: Arc<RwLock<Inner>>,
}

#[derive(Debug, Default)]
struct Inner {
    groups: BTreeMap<ShardGroup, Option<Epoch>>,
}

impl ConsensusEpoch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tracks the shard groups of a newly drawn sync plan. Each starts from the epoch the previous
    /// plan's groups had all reached, since consensus never moves back an epoch.
    pub fn track_groups<I: IntoIterator<Item = ShardGroup>>(&self, shard_groups: I) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let floor = inner.current();
        inner.groups = shard_groups.into_iter().map(|sg| (sg, floor)).collect();
    }

    /// Records that `shard_group` has committed a block in `epoch`. A group outside the current plan
    /// is ignored.
    pub fn observe(&self, shard_group: ShardGroup, epoch: Epoch) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if let Some(observed) = inner.groups.get_mut(&shard_group) {
            *observed = Some(observed.map_or(epoch, |e| e.max(epoch)));
        }
    }

    /// The epoch every shard group has reached, or `None` until each has been observed.
    pub fn current(&self) -> Option<Epoch> {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).current()
    }

    /// The epoch every shard group has reached, held within one epoch below `epoch_manager_epoch`.
    ///
    /// A committee that is executing trails the epoch manager by at most its end-of-epoch block, so
    /// a group observed further behind has stopped and must not hold back the epoch transactions
    /// elsewhere execute in. An epoch past `epoch_manager_epoch` is one this indexer's base layer
    /// scan has not reached and cannot resolve the epoch hash for.
    pub fn current_within(&self, epoch_manager_epoch: Epoch) -> Option<Epoch> {
        self.current()
            .map(|epoch| epoch.clamp(epoch_manager_epoch.saturating_sub(Epoch(1)), epoch_manager_epoch))
    }
}

impl Inner {
    fn current(&self) -> Option<Epoch> {
        let mut min = None::<Epoch>;
        for observed in self.groups.values() {
            let epoch = (*observed)?;
            min = Some(min.map_or(epoch, |m| m.min(epoch)));
        }
        min
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sg(start: u32, end: u32) -> ShardGroup {
        ShardGroup::new(start, end)
    }

    #[test]
    fn it_is_unknown_until_every_group_is_observed() {
        let consensus_epoch = ConsensusEpoch::new();
        consensus_epoch.track_groups([sg(0, 127), sg(128, 255)]);
        consensus_epoch.observe(sg(0, 127), Epoch(5));
        assert_eq!(consensus_epoch.current(), None);
        consensus_epoch.observe(sg(128, 255), Epoch(5));
        assert_eq!(consensus_epoch.current(), Some(Epoch(5)));
    }

    #[test]
    fn it_is_the_lowest_group_epoch() {
        let consensus_epoch = ConsensusEpoch::new();
        consensus_epoch.track_groups([sg(0, 127), sg(128, 255)]);
        consensus_epoch.observe(sg(0, 127), Epoch(6));
        consensus_epoch.observe(sg(128, 255), Epoch(5));
        assert_eq!(consensus_epoch.current(), Some(Epoch(5)));
    }

    #[test]
    fn a_stale_observation_does_not_move_a_group_back() {
        let consensus_epoch = ConsensusEpoch::new();
        consensus_epoch.track_groups([sg(0, 255)]);
        consensus_epoch.observe(sg(0, 255), Epoch(6));
        consensus_epoch.observe(sg(0, 255), Epoch(5));
        assert_eq!(consensus_epoch.current(), Some(Epoch(6)));
    }

    #[test]
    fn a_new_plan_starts_from_the_epoch_already_reached() {
        let consensus_epoch = ConsensusEpoch::new();
        consensus_epoch.track_groups([sg(0, 255)]);
        consensus_epoch.observe(sg(0, 255), Epoch(6));
        consensus_epoch.track_groups([sg(0, 127), sg(128, 255)]);
        assert_eq!(consensus_epoch.current(), Some(Epoch(6)));
        consensus_epoch.observe(sg(0, 127), Epoch(7));
        assert_eq!(consensus_epoch.current(), Some(Epoch(6)));
    }

    #[test]
    fn it_is_held_within_one_epoch_below_the_epoch_manager() {
        let consensus_epoch = ConsensusEpoch::new();
        consensus_epoch.track_groups([sg(0, 127), sg(128, 255)]);
        consensus_epoch.observe(sg(0, 127), Epoch(10));
        consensus_epoch.observe(sg(128, 255), Epoch(4));
        assert_eq!(consensus_epoch.current_within(Epoch(10)), Some(Epoch(9)));

        consensus_epoch.observe(sg(128, 255), Epoch(10));
        assert_eq!(consensus_epoch.current_within(Epoch(9)), Some(Epoch(9)));
        assert_eq!(consensus_epoch.current_within(Epoch(10)), Some(Epoch(10)));
    }

    #[test]
    fn it_ignores_groups_outside_the_plan() {
        let consensus_epoch = ConsensusEpoch::new();
        consensus_epoch.track_groups([sg(0, 255)]);
        consensus_epoch.observe(sg(0, 127), Epoch(6));
        assert_eq!(consensus_epoch.current(), None);
    }
}
