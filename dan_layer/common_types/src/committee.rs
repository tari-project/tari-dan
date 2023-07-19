//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::borrow::Borrow;

use rand::{rngs::OsRng, seq::SliceRandom};
use serde::Serialize;

use crate::{NodeAddressable, ShardId};

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

    /// Returns n - f where n is the number of committee members and f is the tolerated failure nodes.
    pub fn consensus_threshold(&self) -> usize {
        let len = self.members.len();
        if len == 0 {
            return 0;
        }
        let max_failures = (len - 1) / 3;
        len - max_failures
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
#[derive(Debug, Clone, Copy)]
pub struct CommitteeShard {
    num_committees: u32,
    num_members: u32,
    bucket: u32,
}

impl CommitteeShard {
    pub fn new(num_committees: u32, num_members: u32, bucket: u32) -> Self {
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

    pub fn bucket(&self) -> u32 {
        self.bucket
    }

    pub fn includes_shard(&self, shard_id: &ShardId) -> bool {
        let b = shard_id.to_committee_bucket(self.num_committees);
        self.bucket == b
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
}
