//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::Serialize;

use crate::NodeAddressable;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default, Hash)]
pub struct Committee<TAddr> {
    // TODO: encapsulate
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
}

impl<TAddr: NodeAddressable> IntoIterator for Committee<TAddr> {
    type IntoIter = std::vec::IntoIter<Self::Item>;
    type Item = TAddr;

    fn into_iter(self) -> Self::IntoIter {
        self.members.into_iter()
    }
}

impl<TAddr: NodeAddressable> FromIterator<Committee<TAddr>> for Committee<TAddr> {
    fn from_iter<T: IntoIterator<Item = Committee<TAddr>>>(iter: T) -> Self {
        let into_iter = iter.into_iter();
        let (min, maybe_max) = into_iter.size_hint();
        let target_size = maybe_max.unwrap_or(min);
        into_iter.fold(Self::with_capacity(target_size), |mut acc, committee| {
            acc.members.extend(committee.members);
            acc
        })
    }
}
