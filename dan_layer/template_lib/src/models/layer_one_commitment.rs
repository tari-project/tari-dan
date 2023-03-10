use std::fmt::{Display, Formatter};

//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use serde::{Serialize, Deserialize};

use crate::{hash::HashParseError, Hash};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerOneCommitmentAddress(Hash);

impl LayerOneCommitmentAddress {
    pub fn new(hash: Hash) -> Self {
        Self(hash)
    }

    pub fn from_hex(hex: &str) -> Result<Self, HashParseError> {
        Ok(Self(Hash::from_hex(hex)?))
    }

    pub fn try_from_commitment(commitment_bytes: &[u8]) -> Result<Self, HashParseError> {
        Ok(Self(Hash::try_from(commitment_bytes)?))
    }

    pub fn hash(&self) -> &Hash {
        &self.0
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl Display for LayerOneCommitmentAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "commitment_{}", self.0)
    }
}
