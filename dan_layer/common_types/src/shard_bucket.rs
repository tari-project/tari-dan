//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, ops::RangeInclusive};

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

    pub fn to_shard_range(self, num_committees: u32) -> RangeInclusive<ShardId> {
        if num_committees == 0 {
            return RangeInclusive::new(ShardId::zero(), ShardId::from_u256(U256::MAX));
        }
        let bucket_size = U256::MAX / U256::from(num_committees);
        let start = bucket_size * U256::from(self.0);
        let mut end = start + bucket_size;
        // Edge case: The start of the next bucket is excluded except for the last bucket
        if end < U256::MAX {
            end -= U256_ONE;
        }
        RangeInclusive::new(ShardId::from_u256(start), ShardId::from_u256(end))
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
