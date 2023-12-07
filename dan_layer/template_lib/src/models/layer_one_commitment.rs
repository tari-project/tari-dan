//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

use crate::{hash::HashParseError, Hash};

/// The unique identification of a unclaimed confidential output in the Tari network.
/// Used when a user wants to claim burned funds from the Minotari network into the Tari network
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnclaimedConfidentialOutputAddress(Hash);

impl UnclaimedConfidentialOutputAddress {
    pub const fn new(hash: Hash) -> Self {
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

impl TryFrom<&[u8]> for UnclaimedConfidentialOutputAddress {
    type Error = HashParseError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Hash::try_from(value).map(Self)
    }
}

impl Display for UnclaimedConfidentialOutputAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "commitment_{}", self.0)
    }
}
