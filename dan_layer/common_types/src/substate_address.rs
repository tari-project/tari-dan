//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp::Ordering,
    fmt,
    fmt::{Display, Formatter},
    mem::size_of,
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_crypto::tari_utilities::{
    hex::{from_hex, Hex},
    ByteArray,
};
use tari_engine_types::{serde_with, substate::SubstateId, transaction_receipt::TransactionReceiptAddress};
use tari_template_lib::models::ObjectKey;

use crate::{shard::Shard, uint::U256, NumPreshards, ShardGroup};

pub trait ToSubstateAddress {
    fn to_substate_address(&self) -> SubstateAddress;
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct SubstateAddress(
    #[serde(with = "serde_with::hex")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    [u8; SubstateAddress::LENGTH],
);

impl SubstateAddress {
    pub const LENGTH: usize = ObjectKey::LENGTH + size_of::<u32>();

    /// Defines the mapping of SubstateId,version to SubstateAddress
    pub fn from_substate_id(id: &SubstateId, version: u32) -> Self {
        Self::from_object_key(&id.to_object_key(), version)
    }

    pub fn for_transaction_receipt(tx_receipt: TransactionReceiptAddress) -> Self {
        Self::from_substate_id(&tx_receipt.into(), 0)
    }

    pub fn from_object_key(object_key: &ObjectKey, version: u32) -> Self {
        // concatenate (entity_id, component_key), and version
        let mut buf = [0u8; SubstateAddress::LENGTH];
        buf[..ObjectKey::LENGTH].copy_from_slice(object_key);
        buf[ObjectKey::LENGTH..].copy_from_slice(&version.to_be_bytes());

        Self(buf)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FixedHashSizeError> {
        if bytes.len() != SubstateAddress::LENGTH {
            return Err(FixedHashSizeError);
        }
        let key = ObjectKey::try_from(&bytes[..ObjectKey::LENGTH]).map_err(|_| FixedHashSizeError)?;
        let mut v_buf = [0u8; size_of::<u32>()];
        v_buf.copy_from_slice(&bytes[ObjectKey::LENGTH..]);
        let version = u32::from_be_bytes(v_buf);
        Ok(Self::from_object_key(&key, version))
    }

    pub fn is_zero(&self) -> bool {
        self.as_bytes().iter().all(|&b| b == 0)
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

    pub fn from_hash_and_version(hash: FixedHash, version: u32) -> Self {
        // This will cause an error at compile-time if ObjectKey::LENGTH != FixedHash::byte_size()
        // If ObjectKey should differ in length, then this function should likely be removed.
        const _: () = [()][1 - (FixedHash::byte_size() == ObjectKey::LENGTH) as usize];
        let mut buf = [0u8; SubstateAddress::LENGTH];
        buf[..ObjectKey::LENGTH].copy_from_slice(hash.as_bytes());
        buf[ObjectKey::LENGTH..].copy_from_slice(&version.to_be_bytes());
        Self(buf)
    }

    pub fn from_u256_zero_version(address: U256) -> Self {
        Self::from_u256(address, 0)
    }

    pub fn from_u256(address: U256, version: u32) -> Self {
        let mut buf = [0u8; SubstateAddress::LENGTH];
        buf[..ObjectKey::LENGTH].copy_from_slice(&address.to_be_bytes());
        buf[ObjectKey::LENGTH..].copy_from_slice(&version.to_be_bytes());
        Self(buf)
    }

    pub fn object_key_bytes(&self) -> &[u8] {
        &self.0[..ObjectKey::LENGTH]
    }

    pub fn to_version(&self) -> u32 {
        let mut buf = [0u8; size_of::<u32>()];
        buf.copy_from_slice(&self.0[ObjectKey::LENGTH..]);
        u32::from_be_bytes(buf)
    }

    pub fn to_u256(&self) -> U256 {
        let mut buf = [0u8; ObjectKey::LENGTH];
        buf.copy_from_slice(&self.0[..ObjectKey::LENGTH]);
        U256::from_be_bytes(buf)
    }

    /// Calculates and returns the shard number that this SubstateAddress belongs.
    /// A shard is a division of the 256-bit shard space where the boundary of the division if always a power of two.
    pub fn to_shard(&self, num_shards: NumPreshards) -> Shard {
        if num_shards.as_u32() == 1 || self.is_zero() {
            return Shard::from(0u32);
        }
        let addr_u256 = self.to_u256();

        let num_shards = num_shards.as_u32();
        let shard_size = U256::MAX >> num_shards.trailing_zeros();
        Shard::from(
            u32::try_from(addr_u256 / shard_size)
                .expect("to_shard: num_shards is a u32, so this cannot fail")
                .min(num_shards - 1),
        )

        // // 2^15-1 shards with 40 vns per shard = 1,310,680 validators. This limit exists to prevent next_power_of_two
        // // from panicking.
        // let num_shards = num_shards.min(MAX_NUM_SHARDS);
        //
        // // Round down to the next power of two.
        // let next_shards_pow_two = num_shards.next_power_of_two();
        // let half_shard_size = U256::MAX >> next_shards_pow_two.trailing_zeros();
        // let mut next_pow_2_shard =
        //     u32::try_from(addr_u256 / half_shard_size).expect("to_shard: num_shards is a u32, so this cannot fail");
        //
        // // On boundary
        // if addr_u256 % half_shard_size == 0 {
        //     next_pow_2_shard = next_pow_2_shard.saturating_sub(1);
        // }
        //
        // // Half the next power of two i.e. num_shards rounded down to previous power of two
        // let num_shards_pow_two = next_shards_pow_two >> 1;
        // // The "extra" half shards in the space
        // let num_half_shards = num_shards % num_shards_pow_two;
        //
        // // Shard that we would be in if num_shards was a power of two
        // let shard = next_pow_2_shard / 2;
        //
        // // If the shard is higher than all half shards,
        // let shard = if shard >= num_half_shards {
        //     // then add those half shards in
        //     shard + num_half_shards
        // } else {
        //     // otherwise, we use the shard number we'd be in if num_shards was the next power of two
        //     next_pow_2_shard
        // };
        //
        // // u256::MAX address needs to be clamped to the last shard index
        // cmp::min(shard, num_shards - 1).into()
    }

    pub fn to_shard_group(&self, num_shards: NumPreshards, num_committees: u32) -> ShardGroup {
        // number of committees can never exceed number of shards
        let num_committees = num_committees.min(num_shards.as_u32());
        if num_committees <= 1 {
            return ShardGroup::new(Shard::zero(), Shard::from(num_shards.as_u32() - 1));
        }

        let shards_per_committee = num_shards.as_u32() / num_committees;
        let mut shards_per_committee_rem = num_shards.as_u32() % num_committees;

        let shard = self.to_shard(num_shards).as_u32();

        let mut start = 0u32;
        let mut end = shards_per_committee;
        if shards_per_committee_rem > 0 {
            end += 1;
        }
        loop {
            if end > shard {
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

        ShardGroup::new(start, end - 1)
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
    type Error = FixedHashSizeError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Self::from_bytes(&value)
    }
}

impl TryFrom<&[u8]> for SubstateAddress {
    type Error = FixedHashSizeError;

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
    type Err = FixedHashSizeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // TODO: error isnt correct
        let bytes = from_hex(s).map_err(|_| FixedHashSizeError)?;
        Self::from_bytes(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        iter,
        ops::{Bound, RangeBounds, RangeInclusive},
    };

    use rand::{rngs::OsRng, RngCore};

    use super::*;

    #[test]
    fn substate_addresses_to_from_u256_endianness_matches() {
        let mut buf = [0u8; SubstateAddress::LENGTH];
        OsRng.fill_bytes(&mut buf[..ObjectKey::LENGTH]);
        let s = SubstateAddress(buf);
        let result = SubstateAddress::from_u256_zero_version(s.to_u256());
        assert_eq!(result, s);
    }

    #[test]
    fn to_committee_shard_and_shard_range_match() {
        let address = address_at(1, 8);
        let shard = address.to_shard(NumPreshards::P8);
        assert_eq!(shard, 1);

        let range = Shard::from(0).to_substate_address_range(NumPreshards::P2);
        assert_range(range, SubstateAddress::zero()..address_at(1, 2));
        let range = Shard::from(1).to_substate_address_range(NumPreshards::P2);
        assert_range(range, address_at(1, 2)..=SubstateAddress::max());

        for n in 0..7 {
            let range = Shard::from(n).to_substate_address_range(NumPreshards::P8);
            assert_range(range, address_at(n, 8)..address_at(n + 1, 8));
        }

        let range = Shard::from(7).to_substate_address_range(NumPreshards::P8);
        assert_range(range, address_at(7, 8)..=address_at(8, 8));
    }

    // #[test]
    // fn shard_range() {
    //     let range = SubstateAddress::zero().to_address_range(2);
    //     assert_range(range, SubstateAddress::zero()..address_at(1, 2));
    //     let range = SubstateAddress::max().to_address_range(2);
    //     assert_range(range, address_at(1, 2)..=SubstateAddress::max());
    //
    //     // num_shards is a power of two
    //     let power_of_twos =
    //         iter::successors(Some(MAX_NUM_SHARDS.next_power_of_two() >> 1), |&x| Some(x >> 1)).take(15 - 2);
    //     for power_of_two in power_of_twos {
    //         for i in 0..power_of_two {
    //             let range = address_at(i, power_of_two).to_address_range(power_of_two);
    //             if i == 0 {
    //                 assert_range(range, SubstateAddress::zero()..address_at(1, power_of_two));
    //             } else if i >= power_of_two - 1 {
    //                 assert_range(range, address_at(i, power_of_two)..=SubstateAddress::max());
    //             } else {
    //                 assert_range(range, address_at(i, power_of_two)..address_at(i + 1, power_of_two));
    //             }
    //         }
    //     }
    //
    //     // Half shards
    //     let range = address_at(0, 8).to_address_range(6);
    //     assert_range(range, SubstateAddress::zero()..address_at(1, 8));
    //     let range = address_at(1, 8).to_address_range(6);
    //     assert_range(range, address_at(1, 8)..address_at(2, 8));
    //     let range = address_at(2, 8).to_address_range(6);
    //     assert_range(range, address_at(2, 8)..address_at(3, 8));
    //     let range = address_at(3, 8).to_address_range(6);
    //     assert_range(range, address_at(3, 8)..address_at(4, 8));
    //     // Whole shards
    //     let range = address_at(4, 8).to_address_range(6);
    //     assert_range(range, address_at(4, 8)..address_at(6, 8));
    //     let range = address_at(5, 8).to_address_range(6);
    //     assert_range(range, address_at(4, 8)..address_at(6, 8));
    //     let range = address_at(6, 8).to_address_range(6);
    //     assert_range(range, address_at(6, 8)..=SubstateAddress::max());
    //     let range = address_at(7, 8).to_address_range(6);
    //     assert_range(range, address_at(6, 8)..=SubstateAddress::max());
    //     let range = address_at(8, 8).to_address_range(6);
    //     assert_range(range, address_at(6, 8)..=SubstateAddress::max());
    //
    //     let range = plus_one(address_at(1, 4)).to_address_range(20);
    //     // The half shards will split at intervals of END_SHARD_MAX / 32
    //     assert_range(range, address_at(8, 32)..address_at(10, 32));
    //
    //     let range = plus_one(divide_floor(SubstateAddress::max(), 2)).to_address_range(10);
    //     assert_range(range, address_at(8, 16)..address_at(10, 16));
    //
    //     let range = address_at(42, 256).to_address_range(256);
    //     assert_range(range, address_at(42, 256)..address_at(43, 256));
    //     let range = address_at(128, 256).to_address_range(256);
    //     assert_range(range, address_at(128, 256)..address_at(129, 256));
    // }

    #[test]
    fn to_shard() {
        let shard = SubstateAddress::zero().to_shard(NumPreshards::P2);
        assert_eq!(shard, 0);
        let shard = address_at(1, 2).to_shard(NumPreshards::P2);
        assert_eq!(shard, 1);
        let shard = plus_one(address_at(1, 2)).to_shard(NumPreshards::P2);
        assert_eq!(shard, 1);
        let shard = SubstateAddress::max().to_shard(NumPreshards::P2);
        assert_eq!(shard, 1);

        for i in 0..=32 {
            let shard = divide_shard_space(i, 32).to_shard(NumPreshards::P1);
            assert_eq!(shard, 0);
        }

        // 2 shards, exactly half of the physical shard space
        for i in 0..=8 {
            let shard = divide_shard_space(i, 16).to_shard(NumPreshards::P2);
            assert_eq!(shard, 0, "{shard} is not 0 for i: {i}");
        }

        for i in 9..16 {
            let shard = divide_shard_space(i, 16).to_shard(NumPreshards::P2);
            assert_eq!(shard, 1, "{shard} is not 1 for i: {i}");
        }

        // If the number of shards is a power of two, then to_shard should always return the equally divided
        // shard number. We test this for the first u16::MAX power of twos.
        // At boundary
        for power_of_two in iter::successors(Some(1), |&x| Some(x * 2)).take(8) {
            for i in 1..power_of_two {
                let shard = divide_shard_space(i, power_of_two).to_shard(power_of_two.try_into().unwrap());
                assert_eq!(shard, i, "Got: {shard}, Expected: {i} for power_of_two: {power_of_two}");
            }
        }
        // +1 boundary
        for power_of_two in iter::successors(Some(1), |&x| Some(x * 2)).take(8) {
            for i in 0..power_of_two {
                let shard = plus_one(address_at(i, power_of_two)).to_shard(power_of_two.try_into().unwrap());
                assert_eq!(shard, i, "Got: {shard}, Expected: {i} for power_of_two: {power_of_two}");
            }
        }

        let shard = divide_floor(SubstateAddress::max(), 2).to_shard(NumPreshards::P256);
        assert_eq!(shard, 128);
    }

    #[test]
    fn max_committees() {
        let shard = SubstateAddress::max().to_shard(NumPreshards::MAX);
        // When we have n committees, the last committee is n-1.
        assert_eq!(shard, NumPreshards::MAX.as_u32() - 1);
    }

    /// Returns the address of the floor division of the shard space
    fn divide_shard_space(part: u32, of: u32) -> SubstateAddress {
        assert!(part <= of);
        if part == 0 {
            return SubstateAddress::zero();
        }
        if part == of {
            return SubstateAddress::max();
        }
        let size = U256::MAX / U256::from(of);
        SubstateAddress::from_u256_zero_version(U256::from(part) * size)
    }

    /// Returns the start address of the shard with given num_shards
    fn address_at(shard: u32, num_shards: u32) -> SubstateAddress {
        // + shard: For each shard we add 1 to account for remainder
        add(divide_shard_space(shard, num_shards), shard.saturating_sub(1))
    }

    fn divide_floor(shard: SubstateAddress, by: u32) -> SubstateAddress {
        SubstateAddress::from_u256_zero_version(shard.to_u256() / U256::from(by))
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
            assert_eq!(group.as_range(), Shard::from(0)..=Shard::from(1));

            let group = plus_one(address_at(0, 4)).to_shard_group(NumPreshards::P4, 2);
            assert_eq!(group.as_range(), Shard::from(0)..=Shard::from(1));

            let group = address_at(1, 4).to_shard_group(NumPreshards::P4, 2);
            assert_eq!(group.as_range(), Shard::from(0)..=Shard::from(1));

            let group = address_at(2, 4).to_shard_group(NumPreshards::P4, 2);
            assert_eq!(group.as_range(), Shard::from(2)..=Shard::from(3));

            let group = address_at(3, 4).to_shard_group(NumPreshards::P4, 2);
            assert_eq!(group.as_range(), Shard::from(2)..=Shard::from(3));

            let group = SubstateAddress::max().to_shard_group(NumPreshards::P4, 2);
            assert_eq!(group.as_range(), Shard::from(2)..=Shard::from(3));

            let group = minus_one(address_at(1, 64)).to_shard_group(NumPreshards::P64, 16);
            assert_eq!(group.as_range(), Shard::from(0)..=Shard::from(3));
            let group = address_at(4, 64).to_shard_group(NumPreshards::P64, 16);
            assert_eq!(group.as_range(), Shard::from(4)..=Shard::from(7));

            let group = address_at(8, 64).to_shard_group(NumPreshards::P64, 2);
            assert_eq!(group.as_range(), Shard::from(0)..=Shard::from(31));
            let group = address_at(5, 8).to_shard_group(NumPreshards::P64, 2);
            assert_eq!(group.as_range(), Shard::from(32)..=Shard::from(63));

            // On boundary
            let group = address_at(0, 8).to_shard_group(NumPreshards::P64, 2);
            assert_eq!(group.as_range(), Shard::from(0)..=Shard::from(31));
            let group = address_at(4, 8).to_shard_group(NumPreshards::P64, 2);
            assert_eq!(group.as_range(), Shard::from(32)..=Shard::from(63));

            let group = address_at(8, 8).to_shard_group(NumPreshards::P64, 2);
            assert_eq!(group.as_range(), Shard::from(32)..=Shard::from(63));

            let group = plus_one(address_at(3, 64)).to_shard_group(NumPreshards::P64, 32);
            assert_eq!(group.as_range(), Shard::from(2)..=Shard::from(3));

            let group = plus_one(address_at(3, 64)).to_shard_group(NumPreshards::P64, 32);
            assert_eq!(group.as_range(), Shard::from(2)..=Shard::from(3));

            let group = address_at(16, 64).to_shard_group(NumPreshards::P64, 32);
            assert_eq!(group.as_range(), Shard::from(16)..=Shard::from(17));

            let group = minus_one(address_at(1, 4)).to_shard_group(NumPreshards::P64, 64);
            assert_eq!(group.as_range(), Shard::from(16)..=Shard::from(16));

            let group = address_at(66, 256).to_shard_group(NumPreshards::P64, 16);
            assert_eq!(group.as_range(), Shard::from(16)..=Shard::from(19));
        }

        #[test]
        fn it_returns_the_correct_shard_group_generic() {
            let all_num_shards_except_1 = [2, 4, 8, 16, 32, 64, 128, 256]
                .into_iter()
                .map(|n| NumPreshards::try_from(n).unwrap());

            // Note: this test does not calculate the correct assertions if you change this constant.
            const NUM_COMMITTEES: u32 = 2;
            for num_shards in all_num_shards_except_1 {
                for at in 0..num_shards.as_u32() {
                    let group = address_at(at, num_shards.as_u32()).to_shard_group(num_shards, NUM_COMMITTEES);
                    if at < num_shards.as_u32() / NUM_COMMITTEES {
                        assert_eq!(
                            group.as_range(),
                            Shard::from(0)..=Shard::from((num_shards.as_u32() / NUM_COMMITTEES) - 1),
                            "Failed at {at} for num_shards={num_shards}"
                        );
                    } else {
                        assert_eq!(
                            group.as_range(),
                            Shard::from(num_shards.as_u32() / NUM_COMMITTEES)..=Shard::from(num_shards.as_u32() - 1),
                            "Failed at {at} for num_shards={num_shards}"
                        );
                    }
                }
            }
        }

        #[test]
        fn it_returns_the_correct_shard_group_for_odd_num_committees() {
            // All shard groups except the last have 3 shards each

            let group = address_at(0, 64).to_shard_group(NumPreshards::P64, 3);
            // First shard group gets an extra shard to cover the remainder
            assert_eq!(group.as_range(), Shard::from(0)..=Shard::from(21));
            assert_eq!(group.len(), 22);
            let group = address_at(31, 64).to_shard_group(NumPreshards::P64, 3);
            assert_eq!(group.as_range(), Shard::from(22)..=Shard::from(42));
            assert_eq!(group.len(), 21);
            let group = address_at(50, 64).to_shard_group(NumPreshards::P64, 3);
            assert_eq!(group.as_range(), Shard::from(43)..=Shard::from(63));
            assert_eq!(group.len(), 21);

            let group = address_at(3, 64).to_shard_group(NumPreshards::P64, 7);
            assert_eq!(group.as_range(), Shard::from(0)..=Shard::from(9));
            assert_eq!(group.len(), 10);
            let group = address_at(11, 64).to_shard_group(NumPreshards::P64, 7);
            assert_eq!(group.as_range(), Shard::from(10)..=Shard::from(18));
            assert_eq!(group.len(), 9);
            let group = address_at(22, 64).to_shard_group(NumPreshards::P64, 7);
            assert_eq!(group.as_range(), Shard::from(19)..=Shard::from(27));
            assert_eq!(group.len(), 9);
            let group = address_at(60, 64).to_shard_group(NumPreshards::P64, 7);
            assert_eq!(group.as_range(), Shard::from(55)..=Shard::from(63));
            assert_eq!(group.len(), 9);
            let group = address_at(64, 64).to_shard_group(NumPreshards::P64, 7);
            assert_eq!(group.as_range(), Shard::from(55)..=Shard::from(63));
            assert_eq!(group.len(), 9);
            let group = SubstateAddress::zero().to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range(), Shard::from(0)..=Shard::from(2));

            let group = address_at(1, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range(), Shard::from(0)..=Shard::from(2));

            let group = address_at(1, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range(), Shard::from(0)..=Shard::from(2));

            let group = address_at(3, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range(), Shard::from(3)..=Shard::from(5));

            let group = address_at(4, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range(), Shard::from(3)..=Shard::from(5));

            let group = address_at(5, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range(), Shard::from(3)..=Shard::from(5));
            //
            let group = address_at(6, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range(), Shard::from(6)..=Shard::from(7));

            let group = address_at(7, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range(), Shard::from(6)..=Shard::from(7));
            let group = address_at(8, 8).to_shard_group(NumPreshards::P8, 3);
            assert_eq!(group.as_range(), Shard::from(6)..=Shard::from(7));

            // Committee = 5
            let group = address_at(4, 8).to_shard_group(NumPreshards::P8, 5);
            assert_eq!(group.as_range(), Shard::from(4)..=Shard::from(5));

            let group = address_at(7, 8).to_shard_group(NumPreshards::P8, 5);
            assert_eq!(group.as_range(), Shard::from(7)..=Shard::from(7));

            let group = address_at(8, 8).to_shard_group(NumPreshards::P8, 5);
            assert_eq!(group.as_range(), Shard::from(7)..=Shard::from(7));
        }
    }

    mod shard_group_to_substate_address_range {
        use super::*;

        #[test]
        fn it_works() {
            let range = ShardGroup::new(0, 9).to_substate_address_range(NumPreshards::P16);
            assert_range(range, SubstateAddress::zero()..address_at(10, 16));

            let range = ShardGroup::new(1, 15).to_substate_address_range(NumPreshards::P16);
            // Last shard always includes SubstateAddress::max
            assert_range(range, address_at(1, 16)..=address_at(16, 16));

            let range = ShardGroup::new(1, 8).to_substate_address_range(NumPreshards::P16);
            assert_range(range, address_at(1, 16)..address_at(9, 16));

            let range = ShardGroup::new(8, 15).to_substate_address_range(NumPreshards::P16);
            assert_range(range, address_at(8, 16)..=address_at(16, 16));
        }
    }
}
