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
use tari_crypto::tari_utilities::hex::{from_hex, Hex};
use tari_engine_types::{
    hashing::{hasher32, EngineHashDomainLabel},
    serde_with,
    substate::SubstateAddress,
    transaction_receipt::TransactionReceiptAddress,
};

use crate::{shard_bucket::ShardBucket, uint::U256};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ShardId(#[serde(with = "serde_with::hex")] pub [u8; 32]);

impl ShardId {
    /// Defines the mapping of SubstateAddress to ShardId
    pub fn from_address(addr: &SubstateAddress, version: u32) -> Self {
        Self::from_hash(&addr.to_canonical_hash(), version)
    }

    pub fn for_transaction_receipt(tx_receipt: TransactionReceiptAddress) -> Self {
        Self::from_address(&tx_receipt.into(), 0)
    }

    pub fn from_hash(hash: &[u8], version: u32) -> Self {
        let new_addr = hasher32(EngineHashDomainLabel::ShardId)
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

    /// Calculates and returns the bucket number that this ShardId belongs.
    /// A bucket is an equal division of the 256-bit shard space.
    pub fn to_committee_bucket(&self, shards: &Vec<ShardId>, min_committee_size: u32) -> ShardBucket {
        let buckets = shards.len() as u32 / min_committee_size;
        if buckets < 2 {
            return ShardBucket::from(0u32);
        }
        let remainder = shards.len() as u32 % min_committee_size;
        let shard = shards.binary_search(self).map_or_else(|b| b, |b| b) as u32;
        let mut bucket = shard / (min_committee_size + 1);
        if remainder <= bucket {
            bucket = (shard - remainder) / min_committee_size;
        }
        // println!("CIFKO shard: {:2x}, bucket: {:2x}", self.to_u256(), bucket);
        ShardBucket::from(std::cmp::min(bucket as u32, buckets - 1))
    }

    pub fn to_committee_range(&self, shards: &Vec<ShardId>, min_committee_size: u32) -> RangeInclusive<ShardId> {
        let bucket = self.to_committee_bucket(shards, min_committee_size);
        bucket.to_shard_range(shards, min_committee_size)
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
        Some(self.cmp(other))
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

    #[test]
    fn shard_range() {
        let range = divide_floor(ShardId::max(), 2).to_committee_range(3);
        assert_eq!(range, shard(1, 3)..=minus_one(shard(2, 3)));
    }

    #[test]
    fn buckets() {
        let bucket = ShardId::max().to_committee_bucket(0);
        assert_eq!(bucket, 0);
        let bucket = divide_floor(ShardId::max(), 5).to_committee_bucket(20);
        assert_eq!(bucket, 4);
        let bucket = divide_floor(ShardId::max(), 2).to_committee_bucket(10);
        assert_eq!(bucket, 5);
        let bucket = divide_floor(ShardId::max(), 2).to_committee_bucket(256);
        assert_eq!(bucket, 128);
    }

    #[test]
    fn max_committees() {
        let bucket = ShardId::max().to_committee_bucket(u32::MAX);
        assert_eq!(bucket, u32::MAX);
    }

    fn shard(bucket: u32, of: u32) -> ShardId {
        ShardId::from_u256(U256::from(bucket) * (U256::MAX / U256::from(of)))
    }

    fn divide_floor(shard: ShardId, by: u32) -> ShardId {
        ShardId::from_u256(shard.to_u256() / U256::from(by))
    }

    fn minus_one(shard: ShardId) -> ShardId {
        ShardId::from_u256(shard.to_u256() - U256::from(1u32))
    }
}
