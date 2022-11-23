// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

pub mod proto;
pub mod storage;

mod epoch;
pub mod optional;
pub mod serde_with;
mod template_id;

use std::{
    cmp::Ordering,
    fmt,
    fmt::{Display, Formatter},
    str::FromStr,
};

use ::serde::{Deserialize, Serialize};
use borsh::{BorshDeserialize, BorshSerialize};
pub use epoch::Epoch;
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_engine_types::substate::{Substate, SubstateAddress};
use tari_utilities::{
    byte_array::ByteArray,
    hex::{from_hex, Hex},
};
pub use template_id::TemplateId;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, BorshSerialize)]
pub struct ShardId(#[serde(with = "serde_with::hex")] pub [u8; 32]);

impl ShardId {
    pub fn from_address(addr: &SubstateAddress) -> Self {
        match addr {
            SubstateAddress::Component(addr) => addr.into_array().into(),
            SubstateAddress::Resource(addr) => addr.into_array().into(),
            SubstateAddress::Vault(vault_id) => vault_id.into_array().into(),
        }
    }

    pub fn to_le_bytes(&self) -> &[u8] {
        self.0.as_bytes()
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

#[derive(Clone, Debug, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum SubstateChange {
    /// An "Up" state
    Create,
    /// Substate exists but will not be created/destroyed
    Exists,
    /// A "Down" state
    Destroy,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Deserialize, Serialize)]
pub enum SubstateState {
    DoesNotExist,
    Up { created_by: PayloadId, data: Substate },
    Down { deleted_by: PayloadId },
}

impl SubstateState {
    pub fn as_str(&self) -> &str {
        match self {
            Self::DoesNotExist => "DoesNotExist",
            Self::Up { .. } => "Up",
            Self::Down { .. } => "Down",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ObjectClaim {}

impl ObjectClaim {
    pub fn is_valid(&self, _payload: PayloadId) -> bool {
        // TODO: Implement this
        true
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize, Deserialize, Serialize)]
pub struct PayloadId {
    #[serde(with = "serde_with::hex")]
    id: [u8; 32],
}

impl PayloadId {
    pub fn new(id: FixedHash) -> Self {
        let mut v = [0u8; 32];
        v.copy_from_slice(id.as_slice());
        Self { id: v }
    }

    pub const fn zero() -> Self {
        Self { id: [0u8; 32] }
    }

    pub fn as_slice(&self) -> &[u8] {
        self.id.as_slice()
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.as_slice()
    }

    pub fn into_array(self) -> [u8; 32] {
        self.id
    }
}

impl Display for PayloadId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id.to_hex())
    }
}

impl TryFrom<Vec<u8>> for PayloadId {
    type Error = FixedHashSizeError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Ok(PayloadId::new(FixedHash::try_from(value.as_slice())?))
    }
}
