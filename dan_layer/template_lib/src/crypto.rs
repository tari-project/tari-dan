//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

const KEY_LEN: usize = 32;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ed25519PublicKey(#[cfg_attr(feature = "serde", serde(with = "hex::serde"))] [u8; KEY_LEN]);

impl Ed25519PublicKey {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ParseEd25519PublicKeyError> {
        if bytes.len() != KEY_LEN {
            return Err(ParseEd25519PublicKeyError::InvalidLength { size: bytes.len() });
        }

        let mut key = [0u8; KEY_LEN];
        key.copy_from_slice(bytes);
        Ok(Ed25519PublicKey(key))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_array(self) -> [u8; KEY_LEN] {
        self.0
    }
}

impl TryFrom<&[u8]> for Ed25519PublicKey {
    type Error = ParseEd25519PublicKeyError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::from_bytes(value)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseEd25519PublicKeyError {
    InvalidLength { size: usize },
}
