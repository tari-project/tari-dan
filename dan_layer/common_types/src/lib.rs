// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

pub mod proto;
pub mod storage;

mod template_id;

use std::cmp::Ordering;

use borsh::{BorshDeserialize, BorshSerialize};
use tari_common_types::types::FixedHash;
use tari_utilities::byte_array::ByteArray;
pub use template_id::TemplateId;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ObjectId(pub [u8; 32]);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ShardId(pub [u8; 32]);

impl ShardId {
    pub fn to_le_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn new(id: FixedHash) -> Self {
        let mut v = [0u8; 32];
        v.copy_from_slice(id.as_slice());
        Self(v)
    }

    pub fn zero() -> Self {
        Self::new(FixedHash::default())
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

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum SubstateState {
    DoesNotExist,
    Exists { created_by: PayloadId, data: Vec<u8> },
    Destroyed { deleted_by: PayloadId },
}

#[derive(Debug, Clone)]
pub struct ObjectClaim {}

impl ObjectClaim {
    pub fn is_valid(&self, _payload: PayloadId) -> bool {
        true
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize)]
pub struct PayloadId {
    id: [u8; 32],
}

impl PayloadId {
    pub fn new(id: FixedHash) -> Self {
        let mut v = [0u8; 32];
        v.copy_from_slice(id.as_slice());
        Self { id: v }
    }

    pub fn zero() -> Self {
        Self::new(FixedHash::default())
    }

    pub fn as_slice(&self) -> &[u8] {
        self.id.as_slice()
    }
}
