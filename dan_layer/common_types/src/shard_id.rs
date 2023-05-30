//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp::Ordering,
    fmt,
    fmt::{Display, Formatter},
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
        Self(shard.to_le_bytes())
    }

    pub fn to_u256(&self) -> U256 {
        U256::from_le_bytes(self.0)
    }

    /// Calculates and returns the slot number that this ShardId belongs.
    /// A slot is an equal division of the 256-bit shard space. We deterministically assign slots based on the shard id
    /// modulo the number of slots.
    pub fn to_committee_slot(&self, num_slots: u64) -> u64 {
        u64::try_from(self.to_u256() % U256::from(num_slots)).expect("n modulo xu64 is always <= u64::MAX")
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
