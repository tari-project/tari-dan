//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, Bytes};
use tari_template_abi::rust::ops::Deref;

use crate::crypto::InvalidByteLengthError;

#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SchnorrSignatureBytes(#[serde_as(as = "Bytes")] [u8; SchnorrSignatureBytes::length()]);

impl SchnorrSignatureBytes {
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
        Ok(Self(key))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_array(self) -> [u8; Self::length()] {
        self.0
    }
}

impl TryFrom<&[u8]> for SchnorrSignatureBytes {
    type Error = InvalidByteLengthError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::from_bytes(value)
    }
}

impl AsRef<[u8]> for SchnorrSignatureBytes {
    fn as_ref(&self) -> &[u8] {
        self.deref().as_ref()
    }
}

impl Deref for SchnorrSignatureBytes {
    type Target = [u8; Self::length()];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<[u8; SchnorrSignatureBytes::length()]> for SchnorrSignatureBytes {
    fn from(bytes: [u8; SchnorrSignatureBytes::length()]) -> Self {
        Self(bytes)
    }
}
