//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::borrow::Borrow;

use rand::{rngs::OsRng, seq::SliceRandom};
use serde::{Deserialize, Serialize};

use crate::{shard_bucket::ShardBucket, NodeAddressable, ShardId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default, Hash)]
pub struct Committee<TAddr> {
    // TODO: not pub
    pub members: Vec<TAddr>,
}

impl<TAddr: NodeAddressable> Committee<TAddr> {
    pub fn empty() -> Self {
        Self::new(vec![])
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self::new(Vec::with_capacity(cap))
    }

    pub fn new(members: Vec<TAddr>) -> Self {
        Self { members }
    }

    pub fn members(&self) -> &[TAddr] {
        &self.members
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
        self.members.contains(member)
    }

    pub fn shuffle(&mut self) {
        self.members.shuffle(&mut OsRng);
    }

    pub fn calculate_steps_between(&self, member_a: &TAddr, member_b: &TAddr) -> Option<usize> {
        let index_a = self.members.iter().position(|x| x == member_a)? as isize;
        let index_b = self.members.iter().position(|x| x == member_b)? as isize;
        let steps = index_a - index_b;
        if steps < 0 {
            Some((self.members.len() as isize + steps) as usize)
        } else {
            Some(steps as usize)
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &TAddr> {
        self.members.iter()
    }
}

impl<TAddr: NodeAddressable> IntoIterator for Committee<TAddr> {
    type IntoIter = std::vec::IntoIter<Self::Item>;
    type Item = TAddr;

    fn into_iter(self) -> Self::IntoIter {
        self.members.into_iter()
    }
}

impl<'a, TAddr: NodeAddressable> IntoIterator for &'a Committee<TAddr> {
    type IntoIter = std::slice::Iter<'a, TAddr>;
    type Item = &'a TAddr;

    fn into_iter(self) -> Self::IntoIter {
        self.members.iter()
    }
}

impl<TAddr: NodeAddressable> FromIterator<TAddr> for Committee<TAddr> {
    fn from_iter<T: IntoIterator<Item = TAddr>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl<TAddr: NodeAddressable> FromIterator<Committee<TAddr>> for Committee<TAddr> {
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
pub struct CommitteeShard {
    num_committees: u32,
    num_members: u32,
    bucket: ShardBucket,
}

impl CommitteeShard {
    pub fn new(num_committees: u32, num_members: u32, bucket: ShardBucket) -> Self {
        Self {
            num_committees,
            num_members,
            bucket,
        }
    }

    /// Returns n - f where n is the number of committee members and f is the tolerated failure nodes.
    pub fn quorum_threshold(&self) -> u32 {
        let len = self.num_members;
        if len == 0 {
            return 0;
        }
        let max_failures = (len - 1) / 3;
        len - max_failures
    }

    pub fn num_committees(&self) -> u32 {
        self.num_committees
    }

    pub fn num_members(&self) -> u32 {
        self.num_members
    }

    pub fn bucket(&self) -> ShardBucket {
        self.bucket
    }

    pub fn includes_shard(&self, shard_id: &ShardId) -> bool {
        let b = shard_id.to_committee_bucket(self.num_committees);
        self.bucket == b
    }

    pub fn includes_all_shards<I: IntoIterator<Item = B>, B: Borrow<ShardId>>(&self, shard_ids: I) -> bool {
        shard_ids
            .into_iter()
            .all(|shard_id| self.includes_shard(shard_id.borrow()))
    }

    pub fn includes_any_shard<I: IntoIterator<Item = B>, B: Borrow<ShardId>>(&self, shard_ids: I) -> bool {
        shard_ids
            .into_iter()
            .any(|shard_id| self.includes_shard(shard_id.borrow()))
    }

    pub fn filter<'a, I, B: Borrow<ShardId>>(&'a self, items: I) -> impl Iterator<Item = B> + '_
    where I: IntoIterator<Item = B> + 'a {
        items
            .into_iter()
            .filter(|shard_id| self.includes_shard(shard_id.borrow()))
    }

    /// Calculates the number of distinct buckets for a given shard set
    pub fn count_distinct_buckets<'a, I: IntoIterator<Item = &'a ShardId>>(&self, shards: I) -> usize {
        shards
            .into_iter()
            .map(|shard| shard.to_committee_bucket(self.num_committees))
            .collect::<std::collections::HashSet<_>>()
            .len()
    }
}
