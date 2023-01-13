//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::Deref;

use serde::{Deserialize, Serialize};
use tari_bor::borsh::BorshSerialize;
use tari_common_types::types::FixedHash;
use tari_engine_types::hashing::hasher;

use crate::{object_pledge::ObjectPledge, ShardId, TreeNodeHash};

#[derive(Debug, Clone, Deserialize, Serialize, BorshSerialize)]
pub struct ShardPledge {
    pub shard_id: ShardId,
    pub node_hash: TreeNodeHash,
    pub pledge: ObjectPledge,
}

#[derive(Debug, Clone, Deserialize, Serialize, BorshSerialize, Default)]
pub struct ShardPledgeCollection {
    inner: Vec<ShardPledge>,
}

impl ShardPledgeCollection {
    pub fn new(mut pledges: Vec<ShardPledge>) -> Self {
        pledges.sort_by_key(|p| p.shard_id);
        Self { inner: pledges }
    }

    pub const fn empty() -> Self {
        Self { inner: Vec::new() }
    }

    pub fn insert(&mut self, pledge: ShardPledge) {
        let insert_pos = self
            .inner
            .iter()
            .position(|p| p.shard_id >= pledge.shard_id)
            .unwrap_or(self.inner.len() - 1);
        if self.inner[insert_pos].shard_id == pledge.shard_id {
            self.inner[insert_pos] = pledge;
        } else {
            self.inner.insert(insert_pos, pledge);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &ShardPledge> {
        self.inner.iter()
    }

    pub fn pledge_hash(&self) -> FixedHash {
        self.inner
            .iter()
            .map(|p| &p.pledge)
            .fold(hasher("ShardPledgeCollection"), |hasher, p| hasher.chain(p))
            .result()
            .into_array()
            .into()
    }
}

impl FromIterator<ShardPledge> for ShardPledgeCollection {
    fn from_iter<T: IntoIterator<Item = ShardPledge>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl Deref for ShardPledgeCollection {
    type Target = [ShardPledge];

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
