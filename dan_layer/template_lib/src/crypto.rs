//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
// #[cfg(not(feature = "hex"))]
use serde_big_array::BigArray;
use tari_template_abi::rust::{
    fmt::{Display, Formatter},
    string::String,
};

use crate::{models::NonFungibleAddress, Hash};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RistrettoPublicKeyBytes(
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    [u8; RistrettoPublicKeyBytes::length()],
);

impl RistrettoPublicKeyBytes {
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
        Ok(RistrettoPublicKeyBytes(key))
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

    pub fn to_non_fungible_address(&self) -> NonFungibleAddress {
        NonFungibleAddress::from_public_key(*self)
    }
}

impl TryFrom<&[u8]> for RistrettoPublicKeyBytes {
    type Error = InvalidByteLengthError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::from_bytes(value)
    }
}

impl AsRef<[u8]> for RistrettoPublicKeyBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<[u8; 32]> for RistrettoPublicKeyBytes {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidByteLengthError {
    size: usize,
    expected: usize,
}

impl InvalidByteLengthError {
    pub fn actual_size(&self) -> usize {
        self.size
    }

    pub fn to_error_string(&self) -> String {
        format!(
            "Invalid byte length. Expected {} bytes, got {}",
            self.expected, self.size
        )
    }
}

impl Display for InvalidByteLengthError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_error_string())
    }
}

#[cfg(feature = "std")]
impl std::error::Error for InvalidByteLengthError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BalanceProofSignature(
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    //#[cfg_attr(not(feature = "hex"), serde(with = "BigArray"))]
    #[serde(with = "BigArray")] [u8; BalanceProofSignature::length()],
);

impl BalanceProofSignature {
    pub const fn length() -> usize {
        64
    }

    pub fn try_from_parts(public_nonce: &[u8], signature: &[u8]) -> Result<Self, InvalidByteLengthError> {
        if public_nonce.len() != 32 {
            return Err(InvalidByteLengthError {
                size: public_nonce.len(),
                expected: 32,
            });
        }
        if signature.len() != 32 {
            return Err(InvalidByteLengthError {
                size: signature.len(),
                expected: 32,
            });
        }

        let mut key = [0u8; Self::length()];
        key[..32].copy_from_slice(public_nonce);
        key[32..].copy_from_slice(signature);
        Ok(BalanceProofSignature(key))
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
        Ok(BalanceProofSignature(key))
    }

    pub fn as_public_nonce(&self) -> &[u8] {
        &self.0[..32]
    }

    pub fn as_signature(&self) -> &[u8] {
        &self.0[32..]
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_array(self) -> [u8; Self::length()] {
        self.0
    }
}

impl TryFrom<&[u8]> for BalanceProofSignature {
    type Error = InvalidByteLengthError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::from_bytes(value)
    }
}
