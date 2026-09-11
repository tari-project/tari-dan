//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp::Ordering,
    fmt,
    fmt::{Display, Formatter},
    mem::size_of,
    str::FromStr,
};

use borsh::BorshSerialize;
use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_crypto::tari_utilities::hex::Hex;
use tari_engine_types::substate::SubstateId;
use tari_template_lib_types::{Hash32, ObjectKey, TransactionReceiptAddress, hex::fixed_bytes_from_hex};

use crate::{NumPreshards, ShardGroup, shard::Shard, uint::U256};

pub trait ToSubstateAddress {
    fn to_substate_address(&self) -> SubstateAddress;
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, BorshSerialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cbor(transparent)]
pub struct SubstateAddress(
    #[serde(with = "ootle_serde::hex")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[n(0)]
    [u8; SubstateAddress::LENGTH],
);

impl SubstateAddress {
    pub const LENGTH: usize = ObjectKey::LENGTH + size_of::<u64>();

    /// Defines the mapping of SubstateId,version to SubstateAddress
    pub fn from_substate_id(id: &SubstateId, version: u64) -> Self {
        Self::from_object_key(&id.to_object_key(), version)
    }

    pub fn for_transaction_receipt(tx_receipt: TransactionReceiptAddress) -> Self {
        Self::from_substate_id(&tx_receipt.into(), 0)
    }

    pub fn from_object_key(object_key: &ObjectKey, version: u64) -> Self {
        // concatenate (entity_id, component_key), and version
        let mut buf = [0u8; SubstateAddress::LENGTH];
        buf[..ObjectKey::LENGTH].copy_from_slice(object_key);
        buf[ObjectKey::LENGTH..].copy_from_slice(&version.to_be_bytes());

        Self(buf)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SubstateAddressSizeError> {
        if bytes.len() != SubstateAddress::LENGTH {
            return Err(SubstateAddressSizeError::SizeError {
                expected: SubstateAddress::LENGTH,
                actual: bytes.len(),
            });
        }
        let obj_key_bytes = bytes.get(..ObjectKey::LENGTH).expect("length checked");
        let key = ObjectKey::try_from(obj_key_bytes).expect("ObjectKey length is correct");
        let mut v_buf = [0u8; size_of::<u64>()];
        let version_bytes = bytes.get(ObjectKey::LENGTH..).expect("length checked");
        v_buf.copy_from_slice(version_bytes);
        let version = u64::from_be_bytes(v_buf);
        Ok(Self::from_object_key(&key, version))
    }

    pub fn is_zero(&self) -> bool {
        self.as_bytes().iter().all(|&b| b == 0)
    }

    pub fn from_array(array: [u8; SubstateAddress::LENGTH]) -> Self {
        Self(array)
    }

    pub const fn into_array(self) -> [u8; SubstateAddress::LENGTH] {
        self.0
    }

    pub const fn array(&self) -> &[u8; SubstateAddress::LENGTH] {
        &self.0
    }

    pub const fn zero() -> Self {
        Self([0u8; SubstateAddress::LENGTH])
    }

    pub const fn max() -> Self {
        Self([0xffu8; SubstateAddress::LENGTH])
    }

    pub fn from_hash_and_version<T: Into<Hash32>>(hash: T, version: u64) -> Self {
        // This will cause an error at compile-time if ObjectKey::LENGTH != Hash32::LENGTH
        // If ObjectKey should differ in length, then this function should ideally be removed.
        const _: () = [()][1 - (Hash32::LENGTH == ObjectKey::LENGTH) as usize];
        let mut buf = [0u8; SubstateAddress::LENGTH];
        buf[..ObjectKey::LENGTH].copy_from_slice(hash.into().as_slice());
        buf[ObjectKey::LENGTH..].copy_from_slice(&version.to_be_bytes());
        Self(buf)
    }

    pub fn from_u256_zero_version(address: U256) -> Self {
        Self::from_u256(address, 0)
    }

    pub fn from_u256(address: U256, version: u64) -> Self {
        let mut buf = [0u8; SubstateAddress::LENGTH];
        buf[..ObjectKey::LENGTH].copy_from_slice(&address.to_be_bytes());
        buf[ObjectKey::LENGTH..].copy_from_slice(&version.to_be_bytes());
        Self(buf)
    }

    pub fn object_key_bytes(&self) -> &[u8] {
        &self.0[..ObjectKey::LENGTH]
    }

    pub fn to_object_key(&self) -> ObjectKey {
        ObjectKey::try_from(self.object_key_bytes())
            .expect("SubstateAddress: object_key_bytes must return valid ObjectKey bytes")
    }

    pub fn to_u256(&self) -> U256 {
        let mut buf = [0u8; ObjectKey::LENGTH];
        buf.copy_from_slice(self.object_key_bytes());
        U256::from_be_bytes(buf)
    }

    /// Calculates and returns the shard number that this SubstateAddress belongs.
    /// `NumPreshards` is always a power of two, so the 256-bit address space is split into `N = 2^k` equal shards by
    /// reading the top `k` bits of the address as the (zero-indexed) shard number. Shard numbers are one-indexed
    /// because shard 0 is reserved for global substates.
    pub fn to_shard(&self, num_shards: NumPreshards) -> Shard {
        if num_shards.is_one() {
            return Shard::first();
        }
        let shift = u8::BITS - num_shards.as_u32().trailing_zeros();
        let mask = u8::MAX << shift;
        Shard::from_u32(u32::from((self.0[0] & mask) >> shift) + 1)
    }

    pub fn to_shard_group(&self, num_shards: NumPreshards, num_committees: u32) -> ShardGroup {
        // number of committees can never exceed number of shards
        let num_committees = num_committees.min(num_shards.as_u32());
        if num_committees <= 1 {
            return ShardGroup::new(Shard::first(), Shard::from(num_shards.as_u32()));
        }

        let shards_per_committee = num_shards.as_u32() / num_committees;
        let mut shards_per_committee_rem = num_shards.as_u32() % num_committees;

        let shard_index = self.to_shard(num_shards).as_u32() - 1;

        let mut start = 0u32;
        let mut end = shards_per_committee;
        if shards_per_committee_rem > 0 {
            end += 1;
        }
        loop {
            if end > shard_index {
                break;
            }
            start += shards_per_committee;
            if shards_per_committee_rem > 0 {
                start += 1;
                shards_per_committee_rem -= 1;
            }

            end = start + shards_per_committee;
            if shards_per_committee_rem > 0 {
                end += 1;
            }
        }

        ShardGroup::new(start + 1, end)
    }
}

impl From<[u8; SubstateAddress::LENGTH]> for SubstateAddress {
    fn from(bytes: [u8; SubstateAddress::LENGTH]) -> Self {
        Self(bytes)
    }
}

impl From<SubstateAddress> for Vec<u8> {
    fn from(s: SubstateAddress) -> Self {
        s.as_bytes().to_vec()
    }
}

impl TryFrom<Vec<u8>> for SubstateAddress {
    type Error = SubstateAddressSizeError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Self::from_bytes(&value)
    }
}

impl TryFrom<&[u8]> for SubstateAddress {
    type Error = SubstateAddressSizeError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::from_bytes(value)
    }
}

impl AsRef<[u8]> for SubstateAddress {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl PartialOrd for SubstateAddress {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SubstateAddress {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl Display for SubstateAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.to_hex())
    }
}

impl FromStr for SubstateAddress {
    type Err = SubstateAddressSizeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = fixed_bytes_from_hex(s).map_err(|_| SubstateAddressSizeError::FailedToParseHex)?;
        Ok(Self(bytes))
    }
}

impl ToSubstateAddress for SubstateAddress {
    fn to_substate_address(&self) -> SubstateAddress {
        *self
    }
}

impl ToSubstateAddress for &SubstateAddress {
    fn to_substate_address(&self) -> SubstateAddress {
        **self
    }
}

impl ToSubstateAddress for (&SubstateId, u64) {
    fn to_substate_address(&self) -> SubstateAddress {
        SubstateAddress::from_substate_id(self.0, self.1)
    }
}

impl AsRef<SubstateAddress> for SubstateAddress {
    fn as_ref(&self) -> &SubstateAddress {
        self
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SubstateAddressSizeError {
    #[error("Failed to parse SubstateAddress from hex string. Ensure the string is a valid hex and has the correct length of {} characters.", SubstateAddress::LENGTH * 2)]
    FailedToParseHex,
    #[error("Invalid size for SubstateAddress. Expected {expected} bytes, got {actual} bytes.")]
    SizeError { expected: usize, actual: usize },
}

#[cfg(test)]
mod tests {
    use std::{
        iter,
        ops::{Bound, RangeBounds, RangeInclusive},
    };

    use rand::Rng;

    use super::*;

    #[test]
    fn substate_addresses_to_from_u256_endianness_matches() {
        let mut buf = [0u8; SubstateAddress::LENGTH];
        rand::rng().fill_bytes(&mut buf[..ObjectKey::LENGTH]);
        let s = SubstateAddress(buf);
        let result = SubstateAddress::from_u256_zero_version(s.to_u256());
        assert_eq!(result, s);
    }

    #[test]
    fn to_committee_shard_and_shard_range_match() {
        let address = address_at(1, 8);
        let shard = address.to_shard(NumPreshards::P8);
        assert_eq!(shard, 2);

        let range = Shard::global().to_substate_address_range(NumPreshards::P256);
        assert_range(range, SubstateAddress::zero()..=SubstateAddress::max());

        let range = Shard::from(1).to_substate_address_range(NumPreshards::P2);
        assert_range(range, SubstateAddress::zero()..address_at(1, 2));
        let range = Shard::from(2).to_substate_address_range(NumPreshards::P2);
        assert_range(range, address_at(1, 2)..=SubstateAddress::max());

        for n in 0..7 {
            let range = Shard::from(n + 1).to_substate_address_range(NumPreshards::P8);
            assert_range(range, address_at(n, 8)..address_at(n + 1, 8));
        }

        let range = Shard::from(8).to_substate_address_range(NumPreshards::P8);
        assert_range(range, address_at(7, 8)..=address_at(8, 8));
    }

    #[test]
    fn to_shard() {
        let shard = SubstateAddress::zero().to_shard(NumPreshards::P2);
        assert_eq!(shard, 1);
        let shard = minus_one(address_at(1, 2)).to_shard(NumPreshards::P2);
        assert_eq!(shard, 1);
        let shard = address_at(1, 2).to_shard(NumPreshards::P2);
        assert_eq!(shard, 2);
        let shard = plus_one(address_at(1, 2)).to_shard(NumPreshards::P2);
        assert_eq!(shard, 2);
        let shard = SubstateAddress::max().to_shard(NumPreshards::P2);
        assert_eq!(shard, 2);

        for i in 0..=32 {
            let shard = divide_shard_space(i, 32).to_shard(NumPreshards::P1);
            assert_eq!(shard, 1, "failed for shard {}", i);
        }

        // 2 shards, exactly half of the physical shard space. The natural boundary lands at i = 8 (= 2^255) which
        // belongs to shard 2.
        for i in 0..8 {
            let shard = divide_shard_space(i, 16).to_shard(NumPreshards::P2);
            assert_eq!(shard, 1, "{shard} is not 1 for i: {i}");
        }

        for i in 8..16 {
            let shard = divide_shard_space(i, 16).to_shard(NumPreshards::P2);
            assert_eq!(shard, 2, "{shard} is not 2 for i: {i}");
        }

        // If the number of shards is a power of two, then to_shard should always return the equally divided
        // shard number. We test this for the first u16::MAX power of twos.
        // At boundary
        for power_of_two in iter::successors(Some(1), |&x| Some(x * 2)).take(8) {
            for i in 1..power_of_two {
                let shard = divide_shard_space(i, power_of_two).to_shard(power_of_two.try_into().unwrap());
                assert_eq!(
                    shard,
                    i + 1,
                    "Got: {shard}, Expected: {i} for power_of_two: {power_of_two}"
                );
            }
        }
        // +1 boundary
        for power_of_two in iter::successors(Some(1), |&x| Some(x * 2)).take(8) {
            for i in 1..power_of_two {
                let shard = plus_one(address_at(i, power_of_two)).to_shard(power_of_two.try_into().unwrap());
                assert_eq!(
                    shard,
                    i + 1,
                    "Got: {shard}, Expected: {i} for power_of_two: {power_of_two}"
                );
            }
        }

        // Address at the half-way boundary (2^255) is the start of shard 129 when split into 256 equal shards.
        let shard = divide_shard_space(128, 256).to_shard(NumPreshards::P256);
        assert_eq!(shard, 129);
        // The element just below it is the last of shard 128.
        let shard = minus_one(divide_shard_space(128, 256)).to_shard(NumPreshards::P256);
        assert_eq!(shard, 128);
    }

    #[test]
    fn max_committees() {
        let shard = SubstateAddress::max().to_shard(NumPreshards::MAX);
        // When we have n committees, the last shard is n as the zero shard is reserved for global.
        assert_eq!(shard, NumPreshards::MAX.as_u32());
    }

    /// Returns the address `part / of` of the way through the shard space using the natural power-of-two boundary.
    /// `of` must be a power of two.
    fn divide_shard_space(part: u32, of: u32) -> SubstateAddress {
        assert!(part <= of);
        assert!(of.is_power_of_two(), "`of` must be a power of two");
        if part == 0 {
            return SubstateAddress::zero();
        }
        if part == of {
            return SubstateAddress::max();
        }
        let shard_size = (U256::MAX >> of.trailing_zeros()) + U256::ONE;
        SubstateAddress::from_u256_zero_version(U256::from(part) * shard_size)
    }

    /// Returns the start address of the shard with given num_shards
    fn address_at(shard: u32, num_shards: u32) -> SubstateAddress {
        divide_shard_space(shard, num_shards)
    }

    fn minus_one(shard: SubstateAddress) -> SubstateAddress {
        SubstateAddress::from_u256_zero_version(shard.to_u256() - U256::from(1u32))
    }

    fn plus_one(address: SubstateAddress) -> SubstateAddress {
        add(address, 1)
    }

    fn add(address: SubstateAddress, v: u32) -> SubstateAddress {
        SubstateAddress::from_u256_zero_version(address.to_u256().saturating_add(U256::from(v)))
    }

    fn assert_range<R: RangeBounds<SubstateAddress>>(range: RangeInclusive<SubstateAddress>, expected: R) {
        let start = match expected.start_bound() {
            Bound::Included(&start) => start,
            Bound::Excluded(&start) => minus_one(start),
            Bound::Unbounded => panic!("Expected start bound"),
        };

        let end = match expected.end_bound() {
            Bound::Included(&end) => end,
            Bound::Excluded(&end) => minus_one(end),
            Bound::Unbounded => panic!("Expected end bound"),
        };

        assert_eq!(
            range.start().to_u256(),
            start.to_u256(),
            "Start range: Got {} != expected {}",
            range.start(),
            start
        );
        assert_eq!(
            range.end().to_u256(),
            end.to_u256(),
            "End range: Got {} != expected {}",
            range.end(),
            end,
        );
    }

    mod to_shard_group {
        use super::*;

        #[test]
        fn it_returns_the_correct_shard_group() {
            let group = SubstateAddress::zero().to_shard_group(NumPreshards::P4, 2);
            assert_eq!(group.as_range_inclusive(), Shard::from(1)..=Shard::from(2));

            let group = plus_one(address_at(0, 4)).to_shard_group(NumPreshards::P4, 2);
            assert_eq!(group.as_range_inclusive(), Shard::from(1)..=Shard::from(2));

            let group = address_at(1, 4).to_shard_group(NumPreshards::P4, 2);
            assert_eq!(group.as_range_inclusive(), Shard::from(1)..=Shard::from(2));

            let group = address_at(2, 4).to_shard_group(NumPreshards::P4, 2);
            assert_eq!(group.as_range_inclusive(), Shard::from(3)..=Shard::from(4));

            let group = address_at(3, 4).to_shard_group(NumPreshards::P4, 2);
            assert_eq!(group.as_range_inclusive(), Shard::from(3)..=Shard::from(4));

            let group = SubstateAddress::max().to_shard_group(NumPreshards::P4, 2);
            assert_eq!(group.as_range_inclusive(), Shard::from(3)..=Shard::from(4));

            let group = minus_one(address_at(1, 64)).to_shard_group(NumPreshards::P64, 16);
            assert_eq!(group.as_range_inclusive(), Shard::from(1)..=Shard::from(4));
            let group = address_at(4, 64).to_shard_group(NumPreshards::P64, 16);
            assert_eq!(group.as_range_inclusive(), Shard::from(5)..=Shard::from(8));

            let group = address_at(8, 64).to_shard_group(NumPreshards::P64, 2);
            assert_eq!(group.as_range_inclusive(), Shard::from(1)..=Shard::from(32));
            let group = address_at(5, 8).to_shard_group(NumPreshards::P64, 2);
            assert_eq!(group.as_range_inclusive(), Shard::from(33)..=Shard::from(64));

            // On boundary
            let group = address_at(0, 8).to_shard_group(NumPreshards::P64, 2);
            assert_eq!(group.as_range_inclusive(), Shard::from(1)..=Shard::from(32));
            let group = address_at(4, 8).to_shard_group(NumPreshards::P64, 2);
            assert_eq!(group.as_range_inclusive(), Shard::from(33)..=Shard::from(64));

            let group = address_at(8, 8).to_shard_group(NumPreshards::P64, 2);
            assert_eq!(group.as_range_inclusive(), Shard::from(33)..=Shard::from(64));

            let group = plus_one(address_at(3, 64)).to_shard_group(NumPreshards::P64, 32);
            assert_eq!(group.as_range_inclusive(), Shard::from(3)..=Shard::from(4));

            let group = plus_one(address_at(3, 64)).to_shard_group(NumPreshards::P64, 32);
            assert_eq!(group.as_range_inclusive(), Shard::from(3)..=Shard::from(4));

            let group = address_at(16, 64).to_shard_group(NumPreshards::P64, 32);
            assert_eq!(group.as_range_inclusive(), Shard::from(17)..=Shard::from(18));

            // The last address of shard 1-of-4 lies in the 16th of 64 sub-shards.
            let group = minus_one(address_at(1, 4)).to_shard_group(NumPreshards::P64, 64);
            assert_eq!(group.as_range_inclusive(), Shard::from(16)..=Shard::from(16));

            let group = address_at(66, 256).to_shard_group(NumPreshards::P64, 16);
            assert_eq!(group.as_range_inclusive(), Shard::from(17)..=Shard::from(20));
        }

        #[test]
        fn it_returns_the_correct_shard_group_generic() {
            let all_num_shards_except_1 = [2, 4, 8, 16, 32, 64, 128, 256]
                .into_iter()
                .map(|n| NumPreshards::try_from(n).unwrap());

            // Note: this test does not calculate the correct assertions if you change this constant.
            const NUM_COMMITTEES: u32 = 2;
            for num_shards in all_num_shards_except_1 {
                for at in 1..num_shards.as_u32() {
                    let group = address_at(at, num_shards.as_u32()).to_shard_group(num_shards, NUM_COMMITTEES);
                    if at < num_shards.as_u32() / NUM_COMMITTEES {
                        assert_eq!(
                            group.as_range_inclusive(),
                            Shard::from(1)..=Shard::from(num_shards.as_u32() / NUM_COMMITTEES),
                            "Failed at {at} for num_shards={num_shards}"
                        );
                    } else {
                        let range =
                            Shard::from(num_shards.as_u32() / NUM_COMMITTEES + 1)..=Shard::from(num_shards.as_u32());
                        assert_eq!(
                            group.as_range_inclusive(),
                            range,
                            "Failed at {at} for num_shards={num_shards}"
                        );
                    }
                }
            }
        }

        #[test]
        fn it_matches_num_preshard_all_shard_iter() {
            const NUM_COMMITTEES: u32 = 11;
            let groups = (0..NUM_COMMITTEES).map(|i| {
                address_at(i * (256 / NUM_COMMITTEES + 1), 256).to_shard_group(NumPreshards::P256, NUM_COMMITTEES)
            });
            let mut iter = NumPreshards::P256.all_shard_groups_iter(NUM_COMMITTEES);
            let mut total_length = 0;
            for (i, group) in groups.enumerate() {
                assert_eq!(iter.next(), Some(group), "Failed at {group} (i={i})");
                total_length += group.len();
            }
            assert_eq!(iter.next(), None);
            assert_eq!(total_length, 256);
        }

        #[test]
        fn it_returns_the_correct_shard_group_for_odd_num_committees() {
            // All shard groups except the last have 3 shards each

            let group = address_at(0, 64).to_shard_group(NumPreshards::P64, 3);
            // First shard group gets an extra shard to cover the remainder
            assert_eq!(group.as_range_inclusive(), Shard::from(1)..=Shard::from(22));
            assert_eq!(group.len(), 22);
            let group = address_at(31, 64).to_shard_group(NumPreshards::P64, 3);
            assert_eq!(group.as_range_inclusive(), Shard::from(23)..=Shard::from(43));
            assert_eq!(group.len(), 21);
            let group = address_at(50, 64).to_shard_group(NumPreshards::P64, 3);
            assert_eq!(group.as_range_inclusive(), Shard::from(44)..=Shard::from(64));
            assert_eq!(group.len(), 21);

            let group = address_at(3, 64).to_shard_group(NumPreshards::P64, 7);
            assert_eq!(group.as_range_inclusive(), Shard::from(1)..=Shard::from(10));
            assert_eq!(group.len(), 10);
            let group = address_at(11, 64).to_shard_group(NumPreshards::P64, 7);
            assert_eq!(group.as_range_inclusive(), Shard::from(11)..=Shard::from(19));
            assert_eq!(group.len(), 9);
            let group = address_at(22, 64).to_shard_group(NumPreshards::P64, 7);
            assert_eq!(group.as_range_inclusive(), Shard::from(20)..=Shard::from(28));
            assert_eq!(group.len(), 9);
            let group = address_at(60, 64).to_shard_group(NumPreshards::P64, 7);
            assert_eq!(group.as_range_inclusive(), Shard::from(56)..=Shard::from(64));
            assert_eq!(group.len(), 9);
            let group = address_at(64, 64).to_shard_group(NumPreshards::P64, 7);
            assert_eq!(group.as_range_inclusive(), Shard::from(56)..=Shard::from(64));
            assert_eq!(group.len(), 9);
            let group = SubstateAddress::zero().to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range_inclusive(), Shard::from(1)..=Shard::from(3));

            let group = address_at(1, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range_inclusive(), Shard::from(1)..=Shard::from(3));

            let group = address_at(1, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range_inclusive(), Shard::from(1)..=Shard::from(3));

            let group = address_at(3, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range_inclusive(), Shard::from(4)..=Shard::from(6));

            let group = address_at(4, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range_inclusive(), Shard::from(4)..=Shard::from(6));

            let group = address_at(5, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range_inclusive(), Shard::from(4)..=Shard::from(6));
            //
            let group = address_at(6, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range_inclusive(), Shard::from(7)..=Shard::from(8));

            let group = address_at(7, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range_inclusive(), Shard::from(7)..=Shard::from(8));
            let group = address_at(8, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range_inclusive(), Shard::from(7)..=Shard::from(8));

            // Committee = 5
            let group = address_at(4, 8).to_shard_group(NumPreshards::P8, 5);
            assert_eq!(group.as_range_inclusive(), Shard::from(5)..=Shard::from(6));

            let group = address_at(7, 8).to_shard_group(NumPreshards::P8, 5);
            assert_eq!(group.as_range_inclusive(), Shard::from(8)..=Shard::from(8));

            let group = address_at(8, 8).to_shard_group(NumPreshards::P8, 5);
            assert_eq!(group.as_range_inclusive(), Shard::from(8)..=Shard::from(8));
        }
    }

    mod shard_group_to_substate_address_range {
        use super::*;

        #[test]
        fn it_works() {
            let range = ShardGroup::new(1, 9).to_substate_address_range(NumPreshards::P16);
            assert_range(range, SubstateAddress::zero()..address_at(9, 16));

            let range = ShardGroup::new(1, 16).to_substate_address_range(NumPreshards::P16);
            // Last shard always includes SubstateAddress::max
            assert_range(range, address_at(0, 16)..=address_at(16, 16));

            let range = ShardGroup::new(1, 8).to_substate_address_range(NumPreshards::P16);
            assert_range(range, address_at(0, 16)..address_at(8, 16));

            let range = ShardGroup::new(8, 16).to_substate_address_range(NumPreshards::P16);
            assert_range(range, address_at(7, 16)..=address_at(16, 16));
        }
    }

    mod from_str {
        use super::*;

        #[test]
        fn it_works() {
            let s = address_at(1, 8).to_string();
            let parsed = SubstateAddress::from_str(&s).unwrap();
            assert_eq!(parsed, address_at(1, 8));
        }

        #[test]
        fn it_errors_if_too_short() {
            let s = "00";
            let err = SubstateAddress::from_str(s).unwrap_err();
            assert!(matches!(err, SubstateAddressSizeError::FailedToParseHex));
        }
    }
}
