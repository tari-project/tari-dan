//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use core::fmt;
use std::{
    fmt::{Display, Formatter},
    iter,
    ops::RangeInclusive,
    str::FromStr,
};

use borsh::BorshSerialize;
use minicbor::{CborLen, Decode, Decoder, Encode, decode};
use serde::{Deserialize, Deserializer, Serialize};

use crate::{NumPreshards, SubstateAddress, shard::Shard, uint::U256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, BorshSerialize, Encode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ShardGroup {
    #[n(0)]
    start: Shard,
    #[n(1)]
    end_inclusive: Shard,
}

/// The wire shape of a [`ShardGroup`], read before `start <= end_inclusive` is checked.
///
/// `ShardGroup` promises that ordering to every consumer — `len`, `is_empty`, `shard_iter` and
/// `to_substate_address_range` all take it as given — so deserialization upholds it exactly as the
/// constructors do, and a value that violates it is a decode error rather than a `ShardGroup` the
/// rest of the API cannot describe. `Encode` and `CborLen` stay derived on `ShardGroup`, so this
/// mirror must keep the same field indices and types; the round-trip tests enforce that.
#[derive(Deserialize, Decode)]
struct UncheckedShardGroup {
    #[n(0)]
    start: Shard,
    #[n(1)]
    end_inclusive: Shard,
}

fn invalid_bounds_message(start: Shard, end_inclusive: Shard) -> String {
    format!(
        "invalid ShardGroup: start ({}) is greater than end_inclusive ({})",
        start.as_u32(),
        end_inclusive.as_u32()
    )
}

impl<'de> Deserialize<'de> for ShardGroup {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let UncheckedShardGroup { start, end_inclusive } = UncheckedShardGroup::deserialize(deserializer)?;
        Self::new_checked(start, end_inclusive)
            .ok_or_else(|| serde::de::Error::custom(invalid_bounds_message(start, end_inclusive)))
    }
}

impl<'b, C> Decode<'b, C> for ShardGroup {
    fn decode(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Self, decode::Error> {
        let pos = d.position();
        let UncheckedShardGroup { start, end_inclusive } = UncheckedShardGroup::decode(d, ctx)?;
        Self::new_checked(start, end_inclusive)
            .ok_or_else(|| decode::Error::message(invalid_bounds_message(start, end_inclusive)).at(pos))
    }
}

impl ShardGroup {
    const MAX_ENCODED_VALUE: u32 = (NumPreshards::MAX.as_u32() << 16) + NumPreshards::MAX.as_u32();

    /// Creates a new ShardGroup with the given start and end inclusive shards.
    /// ## Panics
    /// Panics if the start shard is greater than the end shard.
    pub fn new<T: Into<Shard> + Copy>(start: T, end_inclusive: T) -> Self {
        Self::new_checked(start, end_inclusive)
            .expect("INVARIANT: start shard must be less than or equal to end_inclusive")
    }

    pub fn new_checked<T: Into<Shard> + Copy>(start: T, end_inclusive: T) -> Option<Self> {
        let start = start.into();
        let end_inclusive = end_inclusive.into();
        if start > end_inclusive {
            return None;
        }
        Some(Self { start, end_inclusive })
    }

    /// Creates a new ShardGroup. The shard group bounds are not checked.
    /// Prepare checked_new unless the bounds have already been checked by the caller.
    pub fn new_unchecked<T: Into<Shard> + Copy>(start: T, end_inclusive: T) -> Self {
        Self {
            start: start.into(),
            end_inclusive: end_inclusive.into(),
        }
    }

    pub fn all_shards(num_preshards: NumPreshards) -> Self {
        Self::new(Shard::first(), Shard::from(num_preshards.as_u32()))
    }

    /// Returns the number of shards in the shard group.
    /// WARN: If the bounds are invalid this will panic/underflow.
    /// If this comes from an untrusted source, `checked_len` should be used to verify the bounds.
    pub const fn len(&self) -> usize {
        (self.end_inclusive.as_u32() + 1 - self.start.as_u32()) as usize
    }

    /// Returns the length of the shard group, or None if the bounds are invalid
    /// The minimum length returned is 1 since the bounds are inclusive.
    pub fn checked_len(&self) -> Option<usize> {
        let len = self
            .end_inclusive
            .as_u32()
            .checked_add(1)?
            .checked_sub(self.start.as_u32())?;
        Some(len as usize).filter(|len| *len > 0)
    }

    pub const fn is_empty(&self) -> bool {
        // Can never be empty because start <= end_inclusive (self.len() >= 1)
        false
    }

    /// Encodes the shard group as a u32. Big endian layout: (start_msb)(start_lsb)(end_msb)(end_lsb).
    /// The maximum shard number is 256 (0x100), so in practise start_msb and end_msb are either 1 or 0.
    pub fn encode_as_u32(&self) -> u32 {
        let mut n = self.start.as_u32() << 16;
        n |= self.end_inclusive.as_u32();
        n
    }

    pub fn decode_from_u32(n: u32) -> Option<Self> {
        if n > Self::MAX_ENCODED_VALUE {
            return None;
        }

        let start = n >> 16;
        let end = n & 0xFFFF;
        Self::new_checked(start, end)
    }

    /// Iterates over every shard in the group. Yields nothing when the bounds are inverted, which
    /// only a [`Self::new_unchecked`] value can be.
    pub fn shard_iter(self) -> impl Iterator<Item = Shard> + 'static {
        (self.start.as_u32()..=self.end_inclusive.as_u32()).map(Shard::from)
    }

    pub fn shard_iter_with_global(self) -> impl Iterator<Item = Shard> + 'static {
        iter::once(Shard::global()).chain(self.shard_iter())
    }

    /// Returns the intersection of two shard groups, if they overlap.
    pub fn intersection(&self, other: &ShardGroup) -> Option<Self> {
        if self.overlaps_shard_group(other) {
            let start = self.start.max(other.start);
            let end_inclusive = self.end_inclusive.min(other.end_inclusive);
            Some(Self::new_unchecked(start, end_inclusive))
        } else {
            None
        }
    }

    pub fn start(&self) -> Shard {
        self.start
    }

    pub fn end(&self) -> Shard {
        self.end_inclusive
    }

    pub fn contains(&self, shard: &Shard) -> bool {
        self.as_range_inclusive().contains(shard)
    }

    pub fn contains_or_global(&self, shard: &Shard) -> bool {
        if shard.is_global() {
            return true;
        }
        self.contains(shard)
    }

    pub fn overlaps_shard_group(&self, other: &ShardGroup) -> bool {
        self.start <= other.end_inclusive && self.end_inclusive >= other.start
    }

    pub const fn as_range_inclusive(&self) -> RangeInclusive<Shard> {
        self.start..=self.end_inclusive
    }

    pub fn to_substate_address_range(self, num_shards: NumPreshards) -> RangeInclusive<SubstateAddress> {
        if num_shards.is_one() {
            return SubstateAddress::zero()..=SubstateAddress::max();
        }

        let num_shards = num_shards.as_u32();
        let shard_size = (U256::MAX >> num_shards.trailing_zeros()) + U256::ONE;
        let start = U256::from(self.start.as_u32() - 1) * shard_size;
        let end = if self.end_inclusive.as_u32() == num_shards {
            SubstateAddress::max()
        } else {
            SubstateAddress::from_u256_zero_version(U256::from(self.end_inclusive.as_u32()) * shard_size - U256::ONE)
        };
        SubstateAddress::from_u256_zero_version(start)..=end
    }

    pub fn to_parsable_string(&self) -> String {
        let mut s = String::new();
        self.write_parsable_string(&mut s).unwrap();
        s
    }

    pub fn write_parsable_string<W: fmt::Write>(&self, f: &mut W) -> fmt::Result {
        write!(f, "{}-{}", self.start.as_u32(), self.end_inclusive.as_u32())
    }
}

impl Display for ShardGroup {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "ShardGroup(")?;
        self.write_parsable_string(f)?;
        write!(f, ")")
    }
}

impl FromStr for ShardGroup {
    type Err = ShardGroupParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('-');
        let start = parts.next().ok_or_else(|| ShardGroupParseError(s.to_string()))?;
        let start = start.parse::<u32>().map_err(|_| ShardGroupParseError(s.to_string()))?;
        let end = parts.next().ok_or_else(|| ShardGroupParseError(s.to_string()))?;
        let end = end.parse::<u32>().map_err(|_| ShardGroupParseError(s.to_string()))?;
        ShardGroup::new_checked(start, end).ok_or_else(|| ShardGroupParseError(s.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Invalid ShardGroup string '{0}'")]
pub struct ShardGroupParseError(pub String);

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
        assert_eq!(
            ShardGroup::decode_from_u32(ShardGroup::MAX_ENCODED_VALUE),
            Some(ShardGroup::new(0x100, 0x100))
        );
        assert_eq!(ShardGroup::decode_from_u32(ShardGroup::MAX_ENCODED_VALUE + 1), None);
        assert_eq!(ShardGroup::decode_from_u32(u32::MAX), None);
    }

    #[test]
    fn it_round_trips_a_valid_group() {
        let sg = ShardGroup::new(10, 20);
        let bytes = tari_bor::encode(&sg).unwrap();
        assert_eq!(tari_bor::decode::<ShardGroup>(&bytes).unwrap(), sg);

        let json = serde_json::to_string(&sg).unwrap();
        assert_eq!(serde_json::from_str::<ShardGroup>(&json).unwrap(), sg);
    }

    #[test]
    fn it_rejects_inverted_bounds_when_deserializing() {
        let inverted = ShardGroup::new_unchecked(5, 2);

        let bytes = tari_bor::encode(&inverted).unwrap();
        tari_bor::decode::<ShardGroup>(&bytes).unwrap_err();

        serde_json::from_str::<ShardGroup>(r#"{"start":5,"end_inclusive":2}"#).unwrap_err();
    }

    #[test]
    fn shard_iter_terminates_on_inverted_bounds() {
        assert_eq!(ShardGroup::new_unchecked(5, 2).shard_iter().count(), 0);
        assert_eq!(ShardGroup::new(5, 7).shard_iter().collect::<Vec<_>>(), vec![
            Shard::from(5),
            Shard::from(6),
            Shard::from(7)
        ]);
    }

    #[test]
    fn to_substate_address_range() {
        let sg = ShardGroup::new(1, 64);
        let range = sg.to_substate_address_range(NumPreshards::P64);
        assert_eq!(*range.start(), SubstateAddress::zero());
        assert_eq!(*range.end(), SubstateAddress::max());
    }

    #[test]
    fn to_string_and_parsing() {
        let sg = ShardGroup::new(0, 63);
        let s = sg.to_parsable_string();
        assert_eq!(s, "0-63");
        let sg2 = s.parse::<ShardGroup>().unwrap();
        assert_eq!(sg, sg2);

        let n = u64::from(u32::MAX) + 1;
        format!("{n}-999").parse::<ShardGroup>().unwrap_err();

        "100-1".parse::<ShardGroup>().unwrap_err();
    }

    #[test]
    fn contains_or_global_works_correctly() {
        let sg = ShardGroup::new(10, 20);

        // Test with a global shard
        let global_shard = Shard::global(); // Assuming this constructor exists
        assert!(sg.contains_or_global(&global_shard));

        // Test with a contained shard
        let contained_shard = Shard::from(15);
        assert!(sg.contains_or_global(&contained_shard));

        // Test with a non-contained shard
        let non_contained_shard = Shard::from(30);
        assert!(!sg.contains_or_global(&non_contained_shard));
    }

    #[test]
    fn all_shards() {
        let sg = ShardGroup::all_shards(NumPreshards::P1);
        assert_eq!(sg, ShardGroup::new(1, 1));
        let sg = ShardGroup::all_shards(NumPreshards::P64);
        assert_eq!(sg, ShardGroup::new(1, 64));
    }

    mod intersection {
        use super::*;

        #[test]
        fn it_calculates_the_intersection_of_overlapping_shard_groups() {
            let sg1 = ShardGroup::new(1, 256);
            let sg2 = ShardGroup::new(10, 20);
            let intersection = sg1.intersection(&sg2).unwrap();
            assert_eq!(intersection, ShardGroup::new(10, 20));

            let sg1 = ShardGroup::new(1, 5);
            let sg2 = ShardGroup::new(3, 7);
            let intersection = sg1.intersection(&sg2).unwrap();
            assert_eq!(intersection, ShardGroup::new(3, 5));
        }

        #[test]
        fn it_returns_none_if_shard_groups_do_not_overlap() {
            let sg1 = ShardGroup::new(1, 5);
            let sg2 = ShardGroup::new(6, 10);
            let intersection = sg1.intersection(&sg2);
            assert!(intersection.is_none());

            let sg1 = ShardGroup::new(1, 5);
            let sg2 = ShardGroup::new(0, 0);
            let intersection = sg1.intersection(&sg2);
            assert!(intersection.is_none());
        }
    }
}
