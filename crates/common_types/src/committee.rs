//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{cmp, fmt::Display, ops::RangeInclusive};

use rand::seq::{IndexedRandom, SliceRandom};
use serde::{Deserialize, Serialize};
use tari_engine_types::substate::SubstateId;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{Epoch, NumPreshards, ShardGroup, SubstateAddress, VersionedSubstateIdRef, VotePower};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default, Hash)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Committee<TAddr> {
    members: Vec<CommitteeMember<TAddr>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct CommitteeMember<TAddr> {
    pub address: TAddr,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: RistrettoPublicKeyBytes,
    pub vote_power: VotePower,
}

impl<TAddr: Display> Display for CommitteeMember<TAddr> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CommitteeMember(addr={}, pk={}, {})",
            self.address, self.public_key, self.vote_power
        )
    }
}

impl<TAddr: PartialEq> Committee<TAddr> {
    pub fn empty() -> Self {
        Self::new(vec![])
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self::new(Vec::with_capacity(cap))
    }

    pub fn new(members: Vec<CommitteeMember<TAddr>>) -> Self {
        Self { members }
    }

    // TODO: remove this - rather create committees from iterators than mutating directly
    pub fn members_mut(&mut self) -> &mut Vec<CommitteeMember<TAddr>> {
        &mut self.members
    }

    pub fn max_failures(&self) -> VotePower {
        let power = self.total_power();
        if power.is_zero() {
            return VotePower::of(0);
        }
        (power - VotePower::of(1)) / VotePower::of(3)
    }

    /// Returns the quorum threshold $n - f$, where $n$ is the committee's total vote power and $f$ is the
    /// tolerated faulty power. This equals $2f + 1$ exactly when $n = 3f + 1$.
    pub fn quorum_threshold(&self) -> VotePower {
        self.total_power() - self.max_failures()
    }

    pub fn total_power(&self) -> VotePower {
        self.members
            .iter()
            .fold(VotePower::default(), |acc, member| acc + member.vote_power)
    }

    pub fn max_node_failures(&self) -> usize {
        // max failures is < 1/3 of the total members
        if self.is_empty() {
            return 0;
        }
        (self.len() + 1) / 3
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn contains(&self, addr: &TAddr) -> bool {
        // TODO(perf); O(n) lookup
        self.members.iter().any(|member| member.address == *addr)
    }

    pub fn contains_public_key(&self, public_key: &RistrettoPublicKeyBytes) -> bool {
        // TODO(perf); O(n) lookup
        self.members.iter().any(|member| *public_key == member.public_key)
    }

    pub fn get_power_by_public_key(&self, public_key: &RistrettoPublicKeyBytes) -> Option<VotePower> {
        // TODO(perf); O(n) lookup
        self.members
            .iter()
            .find(|m| m.public_key == *public_key)
            .map(|m| m.vote_power)
    }

    pub fn get(&self, index: usize) -> Option<&CommitteeMember<TAddr>> {
        self.members.get(index)
    }

    pub fn shuffle(&mut self) {
        self.members.shuffle(&mut rand::rng());
    }

    pub fn shuffled(&self) -> impl Iterator<Item = &CommitteeMember<TAddr>> + '_ {
        self.members.sample(&mut rand::rng(), self.len())
    }

    pub fn select_n_random(&self, n: usize) -> impl Iterator<Item = &TAddr> + '_ {
        self.members.sample(&mut rand::rng(), n).map(|member| &member.address)
    }

    pub fn index_of(&self, member: &TAddr) -> Option<usize> {
        self.members.iter().position(|m| m.address == *member)
    }

    /// Returns the n next members from start_index_inclusive, wrapping around if necessary.
    pub fn select_n_starting_from(&self, n: usize, start_index_inclusive: usize) -> impl Iterator<Item = &TAddr> + '_ {
        let n = cmp::min(n, self.members.len());
        let start_index_inclusive = if self.is_empty() {
            0
        } else {
            start_index_inclusive % self.len()
        };
        self.members
            .iter()
            .map(|m| &m.address)
            .cycle()
            .skip(start_index_inclusive)
            .take(n)
    }

    pub fn calculate_steps_between(&self, member_a: &TAddr, member_b: &TAddr) -> Option<usize> {
        let index_a = self.index_of(member_a)? as isize;
        let index_b = self.index_of(member_b)? as isize;
        let steps = index_a - index_b;
        if steps < 0 {
            Some((self.members.len() as isize + steps) as usize)
        } else {
            Some(steps as usize)
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &CommitteeMember<TAddr>> {
        self.members.iter()
    }

    pub fn address_iter(&self) -> impl Iterator<Item = &TAddr> + '_ {
        self.members.iter().map(|m| &m.address)
    }

    pub fn into_addresses(self) -> impl Iterator<Item = TAddr> {
        self.members.into_iter().map(|m| m.address)
    }

    pub fn public_keys(&self) -> impl Iterator<Item = &RistrettoPublicKeyBytes> {
        self.members.iter().map(|m| &m.public_key)
    }

    pub fn into_public_keys(self) -> impl Iterator<Item = RistrettoPublicKeyBytes> {
        self.members.into_iter().map(|m| m.public_key)
    }
}

impl<TAddr> IntoIterator for Committee<TAddr> {
    type IntoIter = std::vec::IntoIter<Self::Item>;
    type Item = CommitteeMember<TAddr>;

    fn into_iter(self) -> Self::IntoIter {
        self.members.into_iter()
    }
}

impl<'a, TAddr> IntoIterator for &'a Committee<TAddr> {
    type IntoIter = std::slice::Iter<'a, CommitteeMember<TAddr>>;
    type Item = &'a CommitteeMember<TAddr>;

    fn into_iter(self) -> Self::IntoIter {
        self.members.iter()
    }
}

impl<TAddr: PartialEq> FromIterator<CommitteeMember<TAddr>> for Committee<TAddr> {
    fn from_iter<T: IntoIterator<Item = CommitteeMember<TAddr>>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl<TAddr: PartialEq> FromIterator<Committee<TAddr>> for Committee<TAddr> {
    fn from_iter<T: IntoIterator<Item = Committee<TAddr>>>(iter: T) -> Self {
        let into_iter = iter.into_iter();
        let members = into_iter.fold(Vec::new(), |mut acc, committee| {
            acc.extend(committee.members);
            acc
        });

        Self::new(members)
    }
}

/// Represents a "slice" of the 256-bit shard space
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct CommitteeInfo {
    num_shards: NumPreshards,
    num_shard_group_members: u32,
    num_committees: u32,
    shard_group: ShardGroup,
    epoch: Epoch,
    total_power: VotePower,
}

impl CommitteeInfo {
    pub fn new(
        num_shards: NumPreshards,
        num_shard_group_members: u32,
        num_committees: u32,
        shard_group: ShardGroup,
        epoch: Epoch,
        total_power: VotePower,
    ) -> Self {
        Self {
            num_shards,
            num_shard_group_members,
            num_committees,
            shard_group,
            epoch,
            total_power,
        }
    }

    /// Returns the epoch of this CommitteeInfo.
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Returns the quorum threshold $n - f$, where $n$ is the committee's total vote power and $f$ is the
    /// tolerated faulty power. This equals $2f + 1$ exactly when $n = 3f + 1$.
    pub fn quorum_threshold(&self) -> VotePower {
        self.total_power() - self.max_failures()
    }

    /// Returns the maximum faulty vote power $f$ that this committee tolerates.
    pub fn max_failures(&self) -> VotePower {
        if self.total_power().is_zero() {
            return VotePower::zero();
        }
        (self.total_power() - VotePower::of(1)) / VotePower::of(3)
    }

    /// Returns the total voting power of the committee.
    pub fn total_power(&self) -> VotePower {
        self.total_power
    }

    pub fn num_shard_group_members(&self) -> u32 {
        self.num_shard_group_members
    }

    pub fn max_failure_shard_group_members(&self) -> u32 {
        // max failures is < 1/3 of the total members
        // NOTE: this is not to be used as a threshold for quorum.
        (self.num_shard_group_members + 1) / 3
    }

    pub fn num_preshards(&self) -> NumPreshards {
        self.num_shards
    }

    pub fn num_committees(&self) -> u32 {
        self.num_committees
    }

    pub fn shard_group(&self) -> ShardGroup {
        self.shard_group
    }

    pub fn to_substate_address_range(&self) -> RangeInclusive<SubstateAddress> {
        self.shard_group.to_substate_address_range(self.num_shards)
    }

    pub fn includes_substate_address(&self, substate_address: &SubstateAddress) -> bool {
        let s = substate_address.to_shard(self.num_shards);
        self.shard_group.contains(&s)
    }

    pub fn includes_substate_id(&self, substate_id: &SubstateId) -> bool {
        if substate_id.is_global() {
            return true;
        }
        // version doesnt affect shard
        let addr = VersionedSubstateIdRef::new(substate_id, 0);
        let shard = addr.to_shard(self.num_shards);
        self.shard_group.contains(&shard)
    }

    pub fn is_all_local<T: AsRef<SubstateId>, I: IntoIterator<Item = T>>(&self, substate_ids: I) -> bool {
        substate_ids.into_iter().all(|substate_id| {
            let substate_id = substate_id.as_ref();
            if substate_id.is_global() && self.num_committees > 1 {
                return false;
            }
            self.includes_substate_id(substate_id)
        })
    }

    pub fn all_shard_groups_iter(&self) -> impl Iterator<Item = ShardGroup> {
        self.num_shards.all_shard_groups_iter(self.num_committees)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_committee(size: usize) -> Committee<u32> {
        Committee::new(
            (0..size as u32)
                .map(|c| CommitteeMember {
                    address: c,
                    public_key: RistrettoPublicKeyBytes::default(),
                    vote_power: VotePower::of(1),
                })
                .collect(),
        )
    }

    mod select_n_starting_from {
        use super::*;

        #[test]
        fn it_selects_members_wrapping_around() {
            let selected = create_committee(6)
                .select_n_starting_from(6, 4)
                .copied()
                .collect::<Vec<_>>();
            assert_eq!(selected, vec![4, 5, 0, 1, 2, 3]);

            let selected = create_committee(6)
                .select_n_starting_from(3, 6)
                .copied()
                .collect::<Vec<_>>();
            assert_eq!(selected, vec![0, 1, 2]);
        }

        #[test]
        fn it_wraps_the_start_index_around() {
            let selected = create_committee(5)
                .select_n_starting_from(6, 101)
                .copied()
                .collect::<Vec<_>>();
            assert_eq!(selected, vec![1, 2, 3, 4, 0]);
        }

        #[test]
        fn it_wraps_around_once() {
            let selected = create_committee(6)
                .select_n_starting_from(100, 4)
                .copied()
                .collect::<Vec<_>>();
            assert_eq!(selected, vec![4, 5, 0, 1, 2, 3]);
        }

        #[test]
        fn it_does_not_panic_empty_committee() {
            let selected = create_committee(0)
                .select_n_starting_from(6, 4)
                .copied()
                .collect::<Vec<_>>();
            assert!(selected.is_empty());
        }
    }
}
