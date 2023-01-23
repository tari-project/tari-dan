//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, Decode, Encode};
use tari_template_abi::{
    call_engine,
    rust::{fmt, fmt::Display, write},
    EngineOp,
};

use crate::{hash::HashParseError, models::Metadata, Hash};

#[derive(Debug, Clone, Copy, Ord, PartialOrd, PartialEq, Eq, Encode, Decode, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct NftTokenId(Hash);

impl NftTokenId {
    pub fn random() -> Self {
        let uuid = call_engine(EngineOp::GenerateUniqueId, &());
        Self(Hash::try_from_vec(uuid).unwrap())
    }

    pub fn hash(&self) -> &Hash {
        &self.0
    }

    pub fn from_hex(hex: &str) -> Result<Self, HashParseError> {
        let hash = Hash::from_hex(hex)?;
        Ok(Self(hash))
    }
}

impl Display for NftTokenId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "nft_{}", self.0)
    }
}

#[derive(Debug, Clone, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct NftToken {
    metadata: Metadata,
    mutable_data: Vec<u8>,
}

impl NftToken {
    pub fn new(metadata: Metadata, mutable_data: Vec<u8>) -> Self {
        Self { metadata, mutable_data }
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn mutable_data(&self) -> &[u8] {
        &self.mutable_data
    }
}
