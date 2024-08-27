//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

use crate::models::{KeyParseError, ObjectKey};

/// The unique identification of a unclaimed confidential output in the Tari network.
/// Used when a user wants to claim burned funds from the Minotari network into the Tari network
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnclaimedConfidentialOutputAddress(ObjectKey);

impl UnclaimedConfidentialOutputAddress {
    pub fn from_hex(hex: &str) -> Result<Self, KeyParseError> {
        Ok(Self(ObjectKey::from_hex(hex)?))
    }

    pub fn try_from_commitment(commitment_bytes: &[u8]) -> Result<Self, KeyParseError> {
        Ok(Self(ObjectKey::try_from(commitment_bytes)?))
    }

    pub fn as_object_key(&self) -> &ObjectKey {
        &self.0
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl TryFrom<&[u8]> for UnclaimedConfidentialOutputAddress {
    type Error = KeyParseError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        ObjectKey::try_from(value).map(Self)
    }
}

impl Display for UnclaimedConfidentialOutputAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "commitment_{}", self.0)
    }
}
