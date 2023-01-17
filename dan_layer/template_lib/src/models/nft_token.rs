//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, Decode, Encode};
use tari_template_abi::{
    call_engine,
    rust::{fmt, fmt::Display, write},
    EngineOp,
};

use crate::{models::Metadata, Hash};

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
}

impl Display for NftTokenId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0u8; 8];
        buf[0..8].copy_from_slice(&self.0[0..8]);
        let id = u64::from_le_bytes(buf);
        write!(f, "Token: {}", id)
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
