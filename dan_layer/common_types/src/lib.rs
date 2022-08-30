// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

pub mod proto;
pub mod storage;

mod template_id;

use std::cmp::Ordering;

use primitive_types::H256;
use tari_common_types::types::FixedHash;
use tari_utilities::byte_array::ByteArray;
pub use template_id::TemplateId;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ObjectId(pub FixedHash);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ShardId(pub FixedHash);

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

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SubstateChange {
    Create,
    Destroy,
}

#[derive(Debug, Clone)]
pub struct ObjectClaim {}

impl ObjectClaim {
    pub fn is_valid(&self, payload: PayloadId) -> bool {
        true
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PayloadId {
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
