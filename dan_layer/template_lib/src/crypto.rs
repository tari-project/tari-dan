//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

const KEY_LEN: usize = 32;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RistrettoPublicKeyBytes(#[cfg_attr(feature = "serde", serde(with = "hex::serde"))] [u8; KEY_LEN]);

impl RistrettoPublicKeyBytes {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ParseRistrettoPublicKeyError> {
        if bytes.len() != KEY_LEN {
            return Err(ParseRistrettoPublicKeyError::InvalidLength { size: bytes.len() });
        }

        let mut key = [0u8; KEY_LEN];
        key.copy_from_slice(bytes);
        Ok(RistrettoPublicKeyBytes(key))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_array(self) -> [u8; KEY_LEN] {
        self.0
    }
}

impl TryFrom<&[u8]> for RistrettoPublicKeyBytes {
    type Error = ParseRistrettoPublicKeyError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::from_bytes(value)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseRistrettoPublicKeyError {
    InvalidLength { size: usize },
}
