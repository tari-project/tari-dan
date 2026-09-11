//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{error::Error, fmt::Display};

use serde::{Deserialize, Serialize};

use crate::{ShardGroup, shard::Shard};

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumPreshards {
    P1 = 1,
    P2 = 2,
    P4 = 4,
    P8 = 8,
    P16 = 16,
    P32 = 32,
    P64 = 64,
    P128 = 128,
    P256 = 256,
}

impl NumPreshards {
    pub const MAX: Self = Self::P256;
    pub const MAX_SHARD: Shard = Shard::from_u32(Self::MAX.as_u32());

    pub const fn as_u32(self) -> u32 {
        self as u32
    }

    pub const fn num_shards(self) -> usize {
        self as usize
    }

    pub fn is_one(self) -> bool {
        self == Self::P1
    }

    pub fn all_shards_iter(&self) -> impl Iterator<Item = Shard> {
        (1..=self.as_u32()).map(Shard::from)
    }

    pub fn all_shard_groups_iter(&self, num_committees: u32) -> impl Iterator<Item = ShardGroup> {
        let num_shards = self.as_u32();
        let num_shards_per_committee = num_shards / num_committees;
        let mut remainder = num_shards % num_committees;
        let mut start = 0;
        let mut end = num_shards_per_committee - 1;
        std::iter::from_fn(move || {
            if start >= num_shards {
                return None;
            }
            if remainder > 0 {
                end += 1;
                remainder -= 1;
            }
            let group = ShardGroup::new(start + 1, end + 1);
            start = end + 1;
            end = start + num_shards_per_committee - 1;
            Some(group)
        })
    }

    pub const fn current() -> Self {
        Self::P256
    }
}

impl TryFrom<u32> for NumPreshards {
    type Error = InvalidNumPreshards;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::P1),
            2 => Ok(Self::P2),
            4 => Ok(Self::P4),
            8 => Ok(Self::P8),
            16 => Ok(Self::P16),
            32 => Ok(Self::P32),
            64 => Ok(Self::P64),
            128 => Ok(Self::P128),
            256 => Ok(Self::P256),
            _ => Err(InvalidNumPreshards(value)),
        }
    }
}

impl From<NumPreshards> for u32 {
    fn from(num_preshards: NumPreshards) -> u32 {
        num_preshards.as_u32()
    }
}

impl Display for NumPreshards {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug)]
pub struct InvalidNumPreshards(u32);

impl Error for InvalidNumPreshards {}

impl Display for InvalidNumPreshards {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not a valid number of pre-shards", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_calculates_all_shard_groups() {
        let num_preshards = NumPreshards::P256;
        let num_committees = 3;
        let groups: Vec<_> = num_preshards.all_shard_groups_iter(num_committees).collect();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0], ShardGroup::new(1, 86));
        assert_eq!(groups[0].len(), 86);
        assert_eq!(groups[1], ShardGroup::new(87, 171));
        assert_eq!(groups[1].len(), 85);
        assert_eq!(groups[2], ShardGroup::new(172, 256));
        assert_eq!(groups[2].len(), 85);
    }

    #[test]
    fn total_shard_group_lengths_equal_num_preshards() {
        let num_preshards = NumPreshards::P256;
        let num_committees = 234;
        let groups: Vec<_> = num_preshards.all_shard_groups_iter(num_committees).collect();
        let total_length = groups.iter().map(|g| g.len()).sum::<usize>();
        assert_eq!(total_length, num_preshards.as_u32() as usize);
    }
}
