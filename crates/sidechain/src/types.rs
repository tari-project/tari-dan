// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{
    convert::TryFrom,
    fmt::{Display, Formatter},
    ops::{Deref, DerefMut},
};

use borsh::{BorshDeserialize, BorshSerialize};
use digest::{consts::U32, generic_array};
use serde::{Deserialize, Serialize};
use tari_crypto::{
    compressed_key::CompressedKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_utilities::hex::{Hex, HexError};

pub type CompressedPublicKey = CompressedKey<RistrettoPublicKey>;
pub type UncompressedPublicKey = RistrettoPublicKey;
pub type PrivateKey = RistrettoSecretKey;

/// A 32-byte hash. Its serde and Borsh encodings are those of a `[u8; 32]`, the same as the base layer's
/// `tari_common_types::types::FixedHash`, so proofs encoded by either side decode on the other.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Debug,
    Default,
    Hash,
    Deserialize,
    Serialize,
    BorshSerialize,
    BorshDeserialize,
)]
#[serde(transparent)]
pub struct FixedHash([u8; FixedHash::byte_size()]);

impl FixedHash {
    pub const fn new(hash: [u8; FixedHash::byte_size()]) -> Self {
        Self(hash)
    }

    pub const fn byte_size() -> usize {
        32
    }

    pub const fn zero() -> Self {
        Self([0u8; FixedHash::byte_size()])
    }

    pub const fn into_array(self) -> [u8; 32] {
        self.0
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

#[derive(thiserror::Error, Debug)]
#[error("Invalid size")]
pub struct FixedHashSizeError;

impl From<[u8; FixedHash::byte_size()]> for FixedHash {
    fn from(hash: [u8; FixedHash::byte_size()]) -> Self {
        Self(hash)
    }
}

impl From<FixedHash> for [u8; FixedHash::byte_size()] {
    fn from(hash: FixedHash) -> Self {
        hash.0
    }
}

impl TryFrom<Vec<u8>> for FixedHash {
    type Error = FixedHashSizeError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        TryFrom::try_from(value.as_slice())
    }
}

impl TryFrom<&[u8]> for FixedHash {
    type Error = FixedHashSizeError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        let hash = <[u8; FixedHash::byte_size()]>::try_from(bytes).map_err(|_| FixedHashSizeError)?;
        Ok(Self(hash))
    }
}

impl From<generic_array::GenericArray<u8, U32>> for FixedHash {
    fn from(hash: generic_array::GenericArray<u8, U32>) -> Self {
        Self(hash.into())
    }
}

impl PartialEq<[u8]> for FixedHash {
    fn eq(&self, other: &[u8]) -> bool {
        self.0[..].eq(other)
    }
}

impl AsRef<[u8]> for FixedHash {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Hex for FixedHash {
    fn from_hex(hex: &str) -> Result<Self, HexError>
    where Self: Sized {
        let hash = <[u8; FixedHash::byte_size()] as Hex>::from_hex(hex)?;
        Ok(Self(hash))
    }

    fn to_hex(&self) -> String {
        self.0.to_hex()
    }
}

impl Deref for FixedHash {
    type Target = [u8; FixedHash::byte_size()];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FixedHash {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Display for FixedHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}
