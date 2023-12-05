//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, ops::RangeInclusive};

use blake2::digest::typenum::U2;
use serde::{Deserialize, Serialize};

use crate::{
    uint::{U256, U256_ONE},
    ShardId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ShardBucket(u32);

impl ShardBucket {
    pub fn as_u32(self) -> u32 {
        self.0
    }

    pub fn to_shard_range(self, shards: &Vec<ShardId>, min_committee_size: u32) -> RangeInclusive<ShardId> {
        let buckets = shards.len() as u32 / min_committee_size;
        if buckets < 2 {
            return RangeInclusive::new(ShardId::zero(), ShardId::from_u256(U256::MAX));
        }
        let remainder = shards.len() as u32 % min_committee_size;
        let start = if self.0 == 0 {
            ShardId::zero()
        } else {
            ShardId::from_u256(
                shards[(self.0 * min_committee_size + std::cmp::min(remainder, self.0) - 1) as usize].to_u256() +
                    U256_ONE,
            )
        };

        let end = if self.0 == (shards.len() as u32 + min_committee_size - 1) / min_committee_size - 1 {
            ShardId::from_u256(U256::MAX)
        } else {
            shards[((self.0 + 1) * min_committee_size + std::cmp::min(remainder, self.0 + 1) - 1) as usize]
        };
        // println!("CIFKO Bucket {} range: {} - {}", self.0, start, end);
        RangeInclusive::new(start, end)
    }
}

impl From<u32> for ShardBucket {
    fn from(v: u32) -> Self {
        Self(v)
    }
}

impl PartialEq<u32> for ShardBucket {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}
impl PartialEq<ShardBucket> for u32 {
    fn eq(&self, other: &ShardBucket) -> bool {
        *self == other.as_u32()
    }
}

impl Display for ShardBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_u32())
    }
}
