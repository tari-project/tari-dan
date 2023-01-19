use std::fmt::{Display, Formatter};

//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause
use tari_bor::{borsh, Decode, Encode};

use crate::{hash::HashParseError, Hash};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LayerOneCommitmentAddress(Hash);

impl LayerOneCommitmentAddress {
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
        write!(f, "l1_commitment_{}", self.0)
    }
}
