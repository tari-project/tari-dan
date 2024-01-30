//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, Bytes};
use tari_template_abi::rust::fmt::{Display, Formatter};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{crypto::InvalidByteLengthError, Hash};

/// A Pederson Commitment byte contents
#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct PedersonCommitmentBytes(#[serde_as(as = "Bytes")] [u8; PedersonCommitmentBytes::length()]);

impl PedersonCommitmentBytes {
    pub const fn length() -> usize {
        32
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, InvalidByteLengthError> {
        if bytes.len() != Self::length() {
            return Err(InvalidByteLengthError {
                size: bytes.len(),
                expected: Self::length(),
            });
        }

        let mut key = [0u8; Self::length()];
        key.copy_from_slice(bytes);
        Ok(PedersonCommitmentBytes(key))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_array(self) -> [u8; Self::length()] {
        self.0
    }

    pub fn as_hash(&self) -> Hash {
        Hash::from_array(self.0)
    }
}

impl TryFrom<&[u8]> for PedersonCommitmentBytes {
    type Error = InvalidByteLengthError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::from_bytes(value)
    }
}

impl AsRef<[u8]> for PedersonCommitmentBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<[u8; 32]> for PedersonCommitmentBytes {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl Display for PedersonCommitmentBytes {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_hash())
    }
}
