//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, decode_exact, encode, Decode, Encode};
use tari_template_abi::{
    call_engine,
    rust::{fmt, fmt::Display, write},
    EngineOp,
};

use crate::{hash::HashParseError, models::Metadata, Hash};

#[derive(Debug, Clone, Copy, Ord, PartialOrd, PartialEq, Eq, Encode, Decode, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct NonFungibleId(pub Hash);

impl NonFungibleId {
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

impl Display for NonFungibleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "nft_{}", self.0)
    }
}

#[derive(Debug, Clone, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct NonFungible {
    metadata: Metadata,
    mutable_data: Vec<u8>,
}

impl NonFungible {
    pub fn new<T: Encode>(metadata: Metadata, mutable_data: &T) -> Self {
        Self {
            metadata,
            mutable_data: encode(mutable_data).unwrap(),
        }
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn mutable_data(&self) -> &[u8] {
        &self.mutable_data
    }

    pub fn get_data<T: Decode>(&self) -> T {
        decode_exact(&self.mutable_data).expect("Failed to decode NonFungible data")
    }

    pub fn get_data_raw(&self) -> &[u8] {
        &self.mutable_data
    }

    pub fn set_data<T: Encode>(&mut self, data: &T) {
        self.mutable_data = encode(data).unwrap();
    }

    pub fn set_data_raw(&mut self, data: Vec<u8>) {
        self.mutable_data = data;
    }
}
