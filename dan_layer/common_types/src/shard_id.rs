//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp::Ordering,
    fmt,
    fmt::{Display, Formatter},
    ops::RangeInclusive,
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_engine_types::{
    hashing::{hasher, EngineHashDomainLabel},
    serde_with,
    substate::SubstateAddress,
};
use tari_utilities::hex::{from_hex, Hex};

use crate::uint::U256;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ShardId(#[serde(with = "serde_with::hex")] pub [u8; 32]);

impl ShardId {
    /// Defines the mapping of SubstateAddress to ShardId
    pub fn from_address(addr: &SubstateAddress, version: u32) -> Self {
        Self::from_hash(&addr.to_canonical_hash(), version)
    }

    pub fn from_hash(hash: &[u8], version: u32) -> Self {
        let new_addr = hasher(EngineHashDomainLabel::ShardId)
            .chain(&hash)
            .chain(&version)
            .result();
        Self(new_addr.into_array())
    }

    pub fn new(id: FixedHash) -> Self {
        let mut v = [0u8; 32];
        v.copy_from_slice(id.as_slice());
        Self(v)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FixedHashSizeError> {
        FixedHash::try_from(bytes).map(Self::new)
    }

    pub fn into_array(self) -> [u8; 32] {
        self.0
    }

    pub fn zero() -> Self {
        Self::new(FixedHash::default())
    }

    pub fn from_u256(shard: U256) -> Self {
        Self(shard.to_be_bytes())
    }

    pub fn to_u256(&self) -> U256 {
        U256::from_be_bytes(self.0)
    }

    /// Calculates and returns the bucket number that this ShardId belongs.
    /// A bucket is an equal division of the 256-bit shard space.
    pub fn to_committee_bucket(&self, num_committees: u64) -> u64 {
        if num_committees == 0 {
            return 0;
        }
        let bucket_size = U256::MAX / U256::from(num_committees);
        u64::try_from(self.to_u256() / bucket_size).expect("too many committees")
    }

    pub fn to_committee_range(&self, num_committees: u64) -> RangeInclusive<ShardId> {
        if num_committees == 0 {
            return RangeInclusive::new(Self::zero(), Self::from_u256(U256::MAX));
        }
        let bucket_size = U256::MAX / U256::from(num_committees);
        let bucket = self.to_u256() / bucket_size;
        let start = bucket_size * U256::from(bucket);
        let mut end = start + bucket_size;
        // Edge case: The start of the next bucket is excluded except for the last bucket
        if end < U256::MAX {
            end -= U256::from(1u64);
        }
        RangeInclusive::new(Self::from_u256(start), Self::from_u256(end))
    }
}

impl From<[u8; 32]> for ShardId {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl From<ShardId> for Vec<u8> {
    fn from(s: ShardId) -> Self {
        s.as_bytes().to_vec()
    }
}

impl TryFrom<Vec<u8>> for ShardId {
    type Error = FixedHashSizeError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Self::from_bytes(&value)
    }
}

impl TryFrom<&[u8]> for ShardId {
    type Error = FixedHashSizeError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::from_bytes(value)
    }
}

impl AsRef<[u8]> for ShardId {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl PartialOrd for ShardId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl Ord for ShardId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl Display for ShardId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.to_hex())
    }
}

impl FromStr for ShardId {
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
    fn shard_id_to_from_u256_endianness_matches() {
        let mut buf = [0u8; 32];
        OsRng.fill_bytes(&mut buf);
        let s = ShardId(buf);
        let result = ShardId::from_u256(s.to_u256());
        assert_eq!(result, s);
    }
}
