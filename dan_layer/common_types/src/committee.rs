//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Borrow, cmp, ops::RangeInclusive};

use rand::{rngs::OsRng, seq::SliceRandom};
use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_engine_types::substate::SubstateId;

use crate::{shard::Shard, Epoch, NumPreshards, ShardGroup, SubstateAddress};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default, Hash)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct Committee<TAddr> {
    // TODO: not pub
    #[cfg_attr(feature = "ts", ts(type = "Array<[TAddr, string]>"))]
    pub members: Vec<(TAddr, PublicKey)>,
}

impl<TAddr: PartialEq> Committee<TAddr> {
    pub fn empty() -> Self {
        Self::new(vec![])
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self::new(Vec::with_capacity(cap))
    }

    pub fn new(members: Vec<(TAddr, PublicKey)>) -> Self {
        Self { members }
    }

    pub fn members(&self) -> impl Iterator<Item = &TAddr> + '_ {
        self.members.iter().map(|(addr, _)| addr)
    }

    pub fn max_failures(&self) -> usize {
        let len = self.members.len();
        if len == 0 {
            return 0;
        }
        (len - 1) / 3
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn contains(&self, member: &TAddr) -> bool {
        self.members.iter().any(|(addr, _)| addr == member)
    }

    pub fn shuffle(&mut self) {
        self.members.shuffle(&mut OsRng);
    }

    pub fn shuffled(&self) -> impl Iterator<Item = &TAddr> + '_ {
        self.members
            .choose_multiple(&mut OsRng, self.len())
            .map(|(addr, _)| addr)
    }

    pub fn select_n_random(&self, n: usize) -> impl Iterator<Item = &TAddr> + '_ {
        self.members.choose_multiple(&mut OsRng, n).map(|(addr, _)| addr)
    }

    pub fn index_of(&self, member: &TAddr) -> Option<usize> {
        self.members.iter().position(|(addr, _)| addr == member)
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
            .map(|(addr, _)| addr)
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

    pub fn iter(&self) -> impl Iterator<Item = &(TAddr, PublicKey)> {
        self.members.iter()
    }

    pub fn addresses(&self) -> impl Iterator<Item = &TAddr> {
        self.members.iter().map(|(addr, _)| addr)
    }

    pub fn into_addresses(self) -> impl Iterator<Item = TAddr> {
        self.members.into_iter().map(|(addr, _)| addr)
    }

    pub fn public_keys(&self) -> impl Iterator<Item = &PublicKey> {
        self.members.iter().map(|(_, pk)| pk)
    }

    pub fn into_public_keys(self) -> impl Iterator<Item = PublicKey> {
        self.members.into_iter().map(|(_, pk)| pk)
    }

    pub fn contains_public_key(&self, public_key: &PublicKey) -> bool {
        self.members.iter().any(|(_, pk)| pk == public_key)
    }
}

impl<TAddr> IntoIterator for Committee<TAddr> {
    type IntoIter = std::vec::IntoIter<Self::Item>;
    type Item = (TAddr, PublicKey);

    fn into_iter(self) -> Self::IntoIter {
        self.members.into_iter()
    }
}

impl<'a, TAddr> IntoIterator for &'a Committee<TAddr> {
    type IntoIter = std::slice::Iter<'a, (TAddr, PublicKey)>;
    type Item = &'a (TAddr, PublicKey);

    fn into_iter(self) -> Self::IntoIter {
        self.members.iter()
    }
}

impl<TAddr: PartialEq> FromIterator<(TAddr, PublicKey)> for Committee<TAddr> {
    fn from_iter<T: IntoIterator<Item = (TAddr, PublicKey)>>(iter: T) -> Self {
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
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct CommitteeInfo {
    num_shards: NumPreshards,
    num_shard_group_members: u32,
    num_committees: u32,
    shard_group: ShardGroup,
}

impl CommitteeInfo {
    pub fn new(
        num_shards: NumPreshards,
        num_shard_group_members: u32,
        num_committees: u32,
        shard_group: ShardGroup,
    ) -> Self {
        Self {
            num_shards,
            num_shard_group_members,
            num_committees,
            shard_group,
        }
    }

    /// Returns $n - f$ (i.e $2f + 1$) where n is the number of committee members and f is the tolerated failure nodes.
    pub fn quorum_threshold(&self) -> u32 {
        self.num_shard_group_members - self.max_failures()
    }

    /// Returns the maximum number of failures $f$ that can be tolerated by this committee.
    pub fn max_failures(&self) -> u32 {
        let len = self.num_shard_group_members;
        if len == 0 {
            return 0;
        }
        (len - 1) / 3
    }

    pub fn num_shard_group_members(&self) -> u32 {
        self.num_shard_group_members
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
        // version doesnt affect shard
        let addr = SubstateAddress::from_substate_id(substate_id, 0);
        let shard = addr.to_shard(self.num_shards);
        self.shard_group.contains(&shard)
    }

    pub fn includes_all_substate_addresses<I: IntoIterator<Item = B>, B: Borrow<SubstateAddress>>(
        &self,
        substate_addresses: I,
    ) -> bool {
        substate_addresses
            .into_iter()
            .all(|substate_address| self.includes_substate_address(substate_address.borrow()))
    }

    pub fn includes_any_address<I: IntoIterator<Item = B>, B: Borrow<SubstateAddress>>(
        &self,
        substate_addresses: I,
    ) -> bool {
        substate_addresses
            .into_iter()
            .any(|substate_address| self.includes_substate_address(substate_address.borrow()))
    }

    pub fn filter<'a, I, B>(&'a self, items: I) -> impl Iterator<Item = B> + '_
    where
        I: IntoIterator<Item = B> + 'a,
        B: Borrow<SubstateAddress>,
    {
        items
            .into_iter()
            .filter(|substate_address| self.includes_substate_address(substate_address.borrow()))
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct NetworkCommitteeInfo<TAddr> {
    pub epoch: Epoch,
    pub committees: Vec<CommitteeShardInfo<TAddr>>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct CommitteeShardInfo<TAddr> {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub shard: Shard,
    pub substate_address_range: RangeInclusive<SubstateAddress>,
    pub validators: Committee<TAddr>,
}

#[cfg(test)]
mod tests {
    use tari_crypto::ristretto::RistrettoPublicKey;

    use super::*;

    fn create_committee(size: usize) -> Committee<u32> {
        Committee::new((0..size as u32).map(|c| (c, RistrettoPublicKey::default())).collect())
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
