//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::Deref;

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_engine_types::hashing::{hasher, EngineHashDomainLabel, TariHasher};

use crate::{object_pledge::ObjectPledge, ShardId, TreeNodeHash};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ShardPledge {
    pub shard_id: ShardId,
    pub node_hash: TreeNodeHash,
    pub pledge: ObjectPledge,
}

/// An ordered list of ShardPledges.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ShardPledgeCollection {
    pledges: Vec<ShardPledge>,
    pledge_hash: FixedHash,
}

impl ShardPledgeCollection {
    pub fn new(mut pledges: Vec<ShardPledge>) -> Self {
        pledges.sort_by_key(|p| p.shard_id);
        let pledge_hash = hash_pledges(&pledges);
        Self { pledges, pledge_hash }
    }

    pub fn empty() -> Self {
        Self {
            pledges: Vec::new(),
            pledge_hash: hash_pledges(&[]),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &ShardPledge> {
        self.pledges.iter()
    }

    pub fn pledge_hash(&self) -> FixedHash {
        self.pledge_hash
    }
}

fn hash_pledges(pledges: &[ShardPledge]) -> FixedHash {
    pledges
        .iter()
        .map(|p| &p.pledge)
        .fold(hasher(EngineHashDomainLabel::ShardPledgeCollection), TariHasher::chain)
        .result()
        .into_array()
        .into()
}

impl FromIterator<ShardPledge> for ShardPledgeCollection {
    fn from_iter<T: IntoIterator<Item = ShardPledge>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl Deref for ShardPledgeCollection {
    type Target = [ShardPledge];

    fn deref(&self) -> &Self::Target {
        &self.pledges
    }
}
