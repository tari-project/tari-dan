//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
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

use crate::{shard::Shard, uint::U256};

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
            // Non-versionable
            SubstateId::UnclaimedConfidentialOutput(id) => Self::from_hash(*id.hash()),
            SubstateId::TransactionReceipt(id) => Self::from_hash(*id.hash()),
            SubstateId::FeeClaim(id) => Self::from_hash(*id.hash()),
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

    fn from_hash(hash: Hash) -> Self {
        Self(hash.into_array())
    }

    pub fn new(id: [u8; 32]) -> Self {
        Self(id)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FixedHashSizeError> {
        let hash = FixedHash::try_from(bytes)?;
        Ok(Self::new(hash.into_array()))
    }

    pub fn into_array(self) -> [u8; 32] {
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
    /// A shard is an equal division of the 256-bit shard space.
    pub fn to_committee_shard(&self, num_committees: u32) -> Shard {
        if num_committees == 0 {
            return Shard::from(0u32);
        }
        let shard_size = U256::MAX / U256::from(num_committees);
        // 4,294,967,295 committees.
        u32::try_from(self.to_u256() / shard_size)
            .expect("to_committee_shard: num_committees is a u32, so this cannot fail")
            .into()
    }

    pub fn to_committee_range(&self, num_committees: u32) -> RangeInclusive<SubstateAddress> {
        let shard = self.to_committee_shard(num_committees);
        shard.to_substate_address_range(num_committees)
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
        let range = divide_floor(SubstateAddress::max(), 2).to_committee_range(3);
        assert_eq!(range, shard(1, 3)..=minus_one(shard(2, 3)));
    }

    #[test]
    fn shards() {
        let shard = SubstateAddress::max().to_committee_shard(0);
        assert_eq!(shard, 0);
        let shard = divide_floor(SubstateAddress::max(), 5).to_committee_shard(20);
        assert_eq!(shard, 4);
        let shard = divide_floor(SubstateAddress::max(), 2).to_committee_shard(10);
        assert_eq!(shard, 5);
        let shard = divide_floor(SubstateAddress::max(), 2).to_committee_shard(256);
        assert_eq!(shard, 128);
    }

    #[test]
    fn max_committees() {
        let shard = SubstateAddress::max().to_committee_shard(u32::MAX);
        assert_eq!(shard, u32::MAX);
    }

    fn shard(shard: u32, of: u32) -> SubstateAddress {
        SubstateAddress::from_u256(U256::from(shard) * (U256::MAX / U256::from(of)))
    }

    fn divide_floor(shard: SubstateAddress, by: u32) -> SubstateAddress {
        SubstateAddress::from_u256(shard.to_u256() / U256::from(by))
    }

    fn minus_one(shard: SubstateAddress) -> SubstateAddress {
        SubstateAddress::from_u256(shard.to_u256() - U256::from(1u32))
    }
}
