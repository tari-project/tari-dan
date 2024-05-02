//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp,
    cmp::Ordering,
    fmt,
    fmt::{Display, Formatter},
    mem,
    ops::RangeInclusive,
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_crypto::tari_utilities::hex::{from_hex, Hex};
use tari_engine_types::{
    hashing::{hasher32, EngineHashDomainLabel},
    serde_with,
    substate::SubstateId,
    transaction_receipt::TransactionReceiptAddress,
};
use tari_template_lib::{models::ObjectKey, Hash};

use crate::{
    shard::Shard,
    uint::{U256, U256_ZERO},
};

/// This is u16::MAX / 2 as a u32 = 32767 shards. Any number of shards greater than this will be clamped to this value.
/// This is done to limit the number of addresses that are added to the final shard to allow the same shard boundaries.
/// TODO: Change num_shards to a u16
pub(super) const MAX_NUM_SHARDS: u32 = 0x0000_0000_0000_ffff >> 1;

/// Allows the shard space to be divided without any remainder for any 16-bit power of two,
/// so that the address of the shard boundaries is always the same regardless of number of shards.
/// The rest (u16::MAX addresses) of the shard space is added to the last shard.
pub(super) const END_SHARD_MAX: U256 = U256::from_words(u128::MAX, u128::MAX - 0xffff);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct SubstateAddress(
    #[serde(with = "serde_with::hex")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub [u8; 32],
);

impl SubstateAddress {
    /// Defines the mapping of SubstateId to SubstateAddress
    pub fn from_address(id: &SubstateId, version: u32) -> Self {
        match id {
            SubstateId::Component(id) => Self::from_object_key(id.as_object_key(), version),
            SubstateId::Resource(id) => Self::from_object_key(id.as_object_key(), version),
            SubstateId::Vault(id) => Self::from_object_key(id.as_object_key(), version),
            SubstateId::NonFungible(id) => {
                let key = hasher32(EngineHashDomainLabel::NonFungibleId)
                    .chain(id.resource_address())
                    .chain(id.id())
                    .result()
                    .trailing_bytes()
                    .into();

                Self::from_object_key(&ObjectKey::new(id.resource_address().as_entity_id(), key), version)
            },
            SubstateId::NonFungibleIndex(id) => {
                let key = hasher32(EngineHashDomainLabel::NonFungibleIndex)
                    .chain(id.resource_address().as_object_key())
                    .chain(&id.index())
                    .result()
                    .trailing_bytes()
                    .into();
                Self::from_object_key(&ObjectKey::new(id.resource_address().as_entity_id(), key), version)
            },

            // These should only have a version of 0, however the address should account for the version argument passed
            // in. For example, if querying one of these substates with a version > 0 then the substate will not exist.
            SubstateId::UnclaimedConfidentialOutput(id) => Self::from_hash(id.hash(), version),
            SubstateId::TransactionReceipt(id) => Self::from_hash(id.hash(), version),
            SubstateId::FeeClaim(id) => Self::from_hash(id.hash(), version),
        }
    }

    pub fn for_transaction_receipt(tx_receipt: TransactionReceiptAddress) -> Self {
        Self::from_address(&tx_receipt.into(), 0)
    }

    fn from_object_key(object_key: &ObjectKey, version: u32) -> Self {
        // concatenate (entity_id, component_key), and version
        let mut buf = [0u8; 32];
        buf[..ObjectKey::LENGTH].copy_from_slice(object_key);
        buf[ObjectKey::LENGTH..ObjectKey::LENGTH + mem::size_of::<u32>()].copy_from_slice(&version.to_le_bytes());

        Self(buf)
    }

    fn from_hash(hash: &Hash, version: u32) -> Self {
        let new_addr = hasher32(EngineHashDomainLabel::SubstateAddress)
            .chain(hash)
            .chain(&version)
            .result();
        Self(new_addr.into_array())
    }

    pub const fn new(id: [u8; 32]) -> Self {
        Self(id)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FixedHashSizeError> {
        let hash = FixedHash::try_from(bytes)?;
        Ok(Self(hash.into_array()))
    }

    pub const fn into_array(self) -> [u8; 32] {
        self.0
    }

    pub const fn zero() -> Self {
        Self([0u8; 32])
    }

    pub const fn max() -> Self {
        Self([0xffu8; 32])
    }

    pub fn from_u256(shard: U256) -> Self {
        Self(shard.to_be_bytes())
    }

    pub fn to_u256(&self) -> U256 {
        U256::from_be_bytes(self.0)
    }

    /// Calculates and returns the shard number that this SubstateAddress belongs.
    /// A shard is a division of the 256-bit shard space where the boundary of the division if always a power of two.
    pub fn to_committee_shard(&self, num_shards: u32) -> Shard {
        if num_shards == 0 {
            return Shard::from(0u32);
        }
        let addr_u256 = self.to_u256();
        if addr_u256 == U256_ZERO {
            return Shard::from(0u32);
        }

        if num_shards.is_power_of_two() {
            let div = END_SHARD_MAX >> num_shards.trailing_zeros();
            return Shard::from(
                u32::try_from(addr_u256 / div).expect("to_committee_shard: num_shards is a u32, so this cannot fail"),
            );
        }

        // 2^15-1 shards with 40 vns per shard = 1,310,680 validators. This limit exists to prevent next_power_of_two
        // from panicking.
        let num_shards = num_shards.min(MAX_NUM_SHARDS);

        // Round down to the next power of two.
        let next_shards_pow_two = num_shards.next_power_of_two();
        let next_pow_2_shard = u32::try_from(addr_u256 / (END_SHARD_MAX >> next_shards_pow_two.trailing_zeros()))
            .expect("to_committee_shard: num_shards is a u32, so this cannot fail");

        // Half the next power of two i.e. num_shards rounded down to previous power of two
        let num_shards_pow_two = next_shards_pow_two >> 1;
        // The "extra" half shards in the space
        let num_half_shards = num_shards % num_shards_pow_two;

        // Shard that we would be in if num_shards was a power of two
        let shard = next_pow_2_shard / 2;
        // If the shard is higher than all half shards,
        let shard = if shard >= num_half_shards {
            // then add those half shards in
            shard + num_half_shards
        } else {
            // otherwise, we use the shard number we'd be in if num_shards was the next power of two
            next_pow_2_shard
        };

        // u256::MAX address needs to be clamped to the last shard index
        cmp::min(shard, num_shards - 1).into()
    }

    pub fn to_address_range(&self, num_shards: u32) -> RangeInclusive<SubstateAddress> {
        let shard = self.to_committee_shard(num_shards);
        shard.to_substate_address_range(num_shards)
    }
}

impl From<[u8; 32]> for SubstateAddress {
    fn from(bytes: [u8; 32]) -> Self {
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
        ops::{Bound, RangeBounds},
    };

    use rand::{rngs::OsRng, RngCore};

    use super::*;

    #[test]
    fn substate_addresses_to_from_u256_endianness_matches() {
        let mut buf = [0u8; 32];
        OsRng.fill_bytes(&mut buf);
        let s = SubstateAddress(buf);
        let result = SubstateAddress::from_u256(s.to_u256());
        assert_eq!(result, s);
    }

    #[test]
    fn shard_range() {
        let range = address_at(0, 8).to_address_range(2);
        assert_range(range, address_at(0, 2)..address_at(1, 2));

        // num_shards is a power of two
        let power_of_twos =
            iter::successors(Some(MAX_NUM_SHARDS.next_power_of_two() >> 1), |&x| Some(x >> 1)).take(15 - 2);
        for power_of_two in power_of_twos {
            for i in 0..power_of_two {
                let range = address_at(i, power_of_two).to_address_range(power_of_two);
                if i >= power_of_two - 1 {
                    assert_range(range, address_at(i, power_of_two)..=SubstateAddress::max());
                } else {
                    assert_range(range, address_at(i, power_of_two)..address_at(i + 1, power_of_two));
                }
            }
        }

        // Half shards
        let range = plus_one(address_at(0, 8)).to_address_range(6);
        assert_range(range, SubstateAddress::zero()..address_at(1, 8));
        let range = plus_one(address_at(1, 8)).to_address_range(6);
        assert_range(range, address_at(1, 8)..address_at(2, 8));
        let range = plus_one(address_at(2, 8)).to_address_range(6);
        assert_range(range, address_at(2, 8)..address_at(3, 8));
        let range = plus_one(address_at(3, 8)).to_address_range(6);
        assert_range(range, address_at(3, 8)..address_at(4, 8));
        // Whole shards
        let range = plus_one(address_at(4, 8)).to_address_range(6);
        assert_range(range, address_at(4, 8)..address_at(6, 8));
        let range = plus_one(address_at(5, 8)).to_address_range(6);
        assert_range(range, address_at(4, 8)..address_at(6, 8));
        let range = plus_one(address_at(6, 8)).to_address_range(6);
        assert_range(range, address_at(6, 8)..=SubstateAddress::max());
        let range = plus_one(address_at(7, 8)).to_address_range(6);
        assert_range(range, address_at(6, 8)..=SubstateAddress::max());
        let range = plus_one(address_at(8, 8)).to_address_range(6);
        assert_range(range, address_at(6, 8)..=SubstateAddress::max());

        let range = plus_one(address_at(1, 4)).to_address_range(20);
        // The half shards will split at intervals of END_SHARD_MAX / 32
        assert_range(range, address_at(8, 32)..address_at(10, 32));

        let range = divide_floor(SubstateAddress::max(), 2).to_address_range(10);
        assert_range(range, address_at(8, 16)..address_at(10, 16));

        let range = address_at(128, 256).to_address_range(256);
        assert_range(range, address_at(128, 256)..address_at(129, 256));
    }

    #[test]
    fn to_committee_shard() {
        // Edge cases
        let shard = SubstateAddress::max().to_committee_shard(0);
        assert_eq!(shard, 0);
        let shard = SubstateAddress::zero().to_committee_shard(0);
        assert_eq!(shard, 0);
        // let shard = SubstateAddress::max().to_committee_shard(u32::MAX);
        // assert_eq!(shard, u32::MAX - 1);

        for i in 0..32 {
            let shard = address_at(i, 32).to_committee_shard(1);
            assert_eq!(shard, 0);
        }

        for i in 0..8 {
            let shard = address_at(i, 16).to_committee_shard(2);
            assert_eq!(shard, 0, "{shard} is not 0 for i: {i}");
        }

        for i in 8..16 {
            let shard = address_at(i, 16).to_committee_shard(2);
            assert_eq!(shard, 1, "{shard} is not 1 for i: {i}");
        }

        // If the number of shards is a power of two, then to_committee_shard should always return the equally divided
        // shard number. We test this for the first u16::MAX power of twos.
        for power_of_two in iter::successors(Some(1), |&x| Some(x * 2)).take(16) {
            for i in 0..power_of_two {
                let shard = address_at(i, power_of_two).to_committee_shard(power_of_two);
                assert_eq!(shard, i);
            }
        }

        let shard = address_at(0, 8).to_committee_shard(6);
        assert_eq!(shard, 0);
        let shard = minus_one(address_at(1, 8)).to_committee_shard(6);
        assert_eq!(shard, 0);
        let shard = address_at(1, 8).to_committee_shard(6);
        assert_eq!(shard, 1);

        let shard = plus_one(address_at(0, 8)).to_committee_shard(6);
        assert_eq!(shard, 0);
        let shard = plus_one(address_at(1, 8)).to_committee_shard(6);
        assert_eq!(shard, 1);
        let shard = plus_one(address_at(2, 8)).to_committee_shard(6);
        assert_eq!(shard, 2);
        let shard = plus_one(address_at(3, 8)).to_committee_shard(6);
        assert_eq!(shard, 3);
        let shard = plus_one(address_at(4, 8)).to_committee_shard(6);
        assert_eq!(shard, 4);
        let shard = plus_one(address_at(5, 8)).to_committee_shard(6);
        assert_eq!(shard, 4);
        let shard = plus_one(address_at(6, 8)).to_committee_shard(6);
        assert_eq!(shard, 5);
        let shard = plus_one(address_at(7, 8)).to_committee_shard(6);
        assert_eq!(shard, 5);
        let shard = minus_one(address_at(8, 8)).to_committee_shard(6);
        assert_eq!(shard, 5);
        let shard = SubstateAddress::max().to_committee_shard(6);
        assert_eq!(shard, 5);

        let shard = plus_one(address_at(0, 8)).to_committee_shard(5);
        assert_eq!(shard, 0);
        let shard = plus_one(address_at(1, 8)).to_committee_shard(5);
        assert_eq!(shard, 1);
        let shard = plus_one(address_at(2, 8)).to_committee_shard(5);
        assert_eq!(shard, 2);
        let shard = plus_one(address_at(3, 8)).to_committee_shard(5);
        assert_eq!(shard, 2);
        let shard = plus_one(address_at(4, 8)).to_committee_shard(5);
        assert_eq!(shard, 3);
        let shard = plus_one(address_at(5, 8)).to_committee_shard(5);
        assert_eq!(shard, 3);
        let shard = plus_one(address_at(6, 8)).to_committee_shard(5);
        assert_eq!(shard, 4);
        let shard = plus_one(address_at(7, 8)).to_committee_shard(5);
        assert_eq!(shard, 4);
        let shard = minus_one(address_at(8, 8)).to_committee_shard(5);
        assert_eq!(shard, 4);
        let shard = SubstateAddress::max().to_committee_shard(5);
        assert_eq!(shard, 4);

        let shard = plus_one(address_at(1, 4)).to_committee_shard(20);
        // 1/4 * 20 = 5 + 4 half shards in the start of the shard space, - 1 for index
        assert_eq!(shard, 5 + 4 - 1);
        let shard = divide_floor(SubstateAddress::max(), 2).to_committee_shard(10);
        // 8 / 2 = 4 + 2 half shards in the start of the shard space, + 1 boundary of shard, - 1 for index
        assert_eq!(shard, 4 + 2 + 1 - 1);
        let shard = divide_floor(SubstateAddress::max(), 2).to_committee_shard(256);
        assert_eq!(shard, 128);
    }

    fn address_at(shard: u32, of: u32) -> SubstateAddress {
        SubstateAddress::from_u256(U256::from(shard) * (END_SHARD_MAX / U256::from(of)))
    }

    fn divide_floor(shard: SubstateAddress, by: u32) -> SubstateAddress {
        SubstateAddress::from_u256(shard.to_u256() / U256::from(by))
    }

    fn minus_one(shard: SubstateAddress) -> SubstateAddress {
        SubstateAddress::from_u256(shard.to_u256() - U256::from(1u32))
    }

    fn plus_one(shard: SubstateAddress) -> SubstateAddress {
        SubstateAddress::from_u256(shard.to_u256() + U256::from(1u32))
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
}
