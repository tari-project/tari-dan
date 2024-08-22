//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Display, Formatter},
    iter,
    ops::RangeInclusive,
};

use serde::{Deserialize, Serialize};

use crate::{shard::Shard, uint::U256, NumPreshards, SubstateAddress};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct ShardGroup {
    start: Shard,
    end_inclusive: Shard,
}

impl ShardGroup {
    pub fn new<T: Into<Shard> + Copy>(start: T, end_inclusive: T) -> Self {
        let start = start.into();
        let end_inclusive = end_inclusive.into();
        assert!(
            start <= end_inclusive,
            "INVARIANT: start shard must be less than or equal to end_inclusive"
        );
        Self { start, end_inclusive }
    }

    pub fn all_shards(num_preshards: NumPreshards) -> Self {
        Self::new(Shard::zero(), Shard::from(num_preshards.as_u32() - 1))
    }

    pub const fn len(&self) -> usize {
        (self.end_inclusive.as_u32() + 1 - self.start.as_u32()) as usize
    }

    pub const fn is_empty(&self) -> bool {
        // Can never be empty because start <= end_inclusive (self.len() >= 1)
        false
    }

    /// Encodes the shard group as a u32. Little endian layout: (0)(0)(start)(end).
    pub fn encode_as_u32(&self) -> u32 {
        // ShardGroup fits into a u16 because even for max NumPreshards (NumPreshards::TwoFiftySix), the maximum
        // possible shard is 255 (the first shard is 0) We therefore encode it into the last two (LS)
        // bytes of the u32. The first two (MS) bytes of the u32 are always 0.
        //
        // A u32 is used because there is no reason not to, and it may give some wiggle room for potential future data
        // to be encoded without any performance difference on most architectures.
        let mut n = self.start.as_u32() << 8;
        n |= self.end_inclusive.as_u32();
        n
    }

    pub fn decode_from_u32(n: u32) -> Option<Self> {
        if n > 0xFFFF {
            return None;
        }

        let start = n >> 8;
        let end = n & 0xFF;
        Some(Self::new(start, end))
    }

    pub fn shard_iter(&self) -> impl Iterator<Item = Shard> + '_ {
        iter::successors(Some(self.start), move |&shard| {
            if shard == self.end_inclusive {
                None
            } else {
                Some(Shard::from(shard.as_u32() + 1))
            }
        })
    }

    pub fn start(&self) -> Shard {
        self.start
    }

    pub fn end(&self) -> Shard {
        self.end_inclusive
    }

    pub fn contains(&self, shard: &Shard) -> bool {
        self.as_range().contains(shard)
    }

    pub fn as_range(&self) -> RangeInclusive<Shard> {
        self.start..=self.end_inclusive
    }

    pub fn to_substate_address_range(self, num_shards: NumPreshards) -> RangeInclusive<SubstateAddress> {
        if num_shards.is_one() {
            return SubstateAddress::zero()..=SubstateAddress::max();
        }

        let shard_size = U256::MAX >> num_shards.as_u32().trailing_zeros();
        let start = if self.start.is_zero() {
            U256::ZERO
        } else {
            shard_size * U256::from(self.start.as_u32()) + U256::from(self.start.as_u32() - 1)
        };
        if self.end_inclusive == num_shards.as_u32() - 1 {
            return SubstateAddress::from_u256_zero_version(start)..=SubstateAddress::max();
        }

        let end =
            shard_size * U256::from(self.end_inclusive.as_u32()) + shard_size + U256::from(self.end_inclusive.as_u32());
        SubstateAddress::from_u256_zero_version(start)..=SubstateAddress::from_u256_zero_version(end - 1)
    }
}

impl Display for ShardGroup {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ShardGroup[{}, {}]", self.start, self.end_inclusive)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn encode_decode() {
        let sg = ShardGroup::new(123, 234);
        let n = sg.encode_as_u32();
        let sg2 = ShardGroup::decode_from_u32(n).unwrap();
        assert_eq!(sg, sg2);
        assert_eq!(ShardGroup::decode_from_u32(0), Some(ShardGroup::new(0, 0)));
        assert_eq!(ShardGroup::decode_from_u32(0xFFFF), Some(ShardGroup::new(0xFF, 0xFF)));
        assert_eq!(ShardGroup::decode_from_u32(0xFFFF + 1), None);
        assert_eq!(ShardGroup::decode_from_u32(u32::MAX), None);
    }

    #[test]
    fn to_substate_address_range() {
        let sg = ShardGroup::new(0, 63);
        let range = sg.to_substate_address_range(NumPreshards::P64);
        assert_eq!(*range.start(), SubstateAddress::zero());
        assert_eq!(*range.end(), SubstateAddress::max());
    }
}
