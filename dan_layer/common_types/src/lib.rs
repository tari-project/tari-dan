// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

pub mod proto;
pub mod storage;

mod template_id;

use std::cmp::Ordering;

use serde::{Deserialize, Deserializer};
use tari_common_types::types::FixedHash;
use tari_utilities::{byte_array::ByteArray, hex::Hex};
pub use template_id::TemplateId;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
pub struct ObjectId(#[serde(deserialize_with = "deserialize_fixed_hash_from_hex")] pub FixedHash);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Deserialize)]
pub struct ShardId(#[serde(deserialize_with = "deserialize_fixed_hash_from_hex")] pub FixedHash);

impl ShardId {
    pub fn to_le_bytes(&self) -> &[u8] {
        self.0.as_bytes()
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

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
pub enum SubstateChange {
    Create,
    Destroy,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObjectClaim {}

impl ObjectClaim {
    pub fn is_valid(&self, _payload: PayloadId) -> bool {
        true
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
pub struct PayloadId {
    #[serde(deserialize_with = "deserialize_fixed_hash_from_hex")]
    id: FixedHash,
}

impl PayloadId {
    pub fn new(id: FixedHash) -> Self {
        Self { id }
    }

    pub fn zero() -> Self {
        Self { id: FixedHash::zero() }
    }

    pub fn as_slice(&self) -> &[u8] {
        self.id.as_slice()
    }
}

/// Use a serde deserializer to serialize the hex string of the given object.
pub fn deserialize_fixed_hash_from_hex<'de, D>(deserializer: D) -> Result<FixedHash, D::Error>
where D: Deserializer<'de> {
    let hex = String::deserialize(deserializer)?;
    FixedHash::from_hex(&hex).map_err(serde::de::Error::custom)
}
