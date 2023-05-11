//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    convert::TryFrom,
    fmt::{Display, Formatter},
};

use digest::{consts::U32, generic_array};
use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_engine_types::serde_with;
use tari_utilities::hex::{Hex, HexError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct TreeNodeHash(#[serde(with = "serde_with::hex")] FixedHash);

impl TreeNodeHash {
    pub fn zero() -> Self {
        Self(FixedHash::zero())
    }

    pub fn is_zero(&self) -> bool {
        *self == Self::zero()
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl From<[u8; FixedHash::byte_size()]> for TreeNodeHash {
    fn from(hash: [u8; FixedHash::byte_size()]) -> Self {
        Self(hash.into())
    }
}

impl From<generic_array::GenericArray<u8, U32>> for TreeNodeHash {
    fn from(hash: digest::generic_array::GenericArray<u8, U32>) -> Self {
        Self(hash.into())
    }
}

impl TryFrom<Vec<u8>> for TreeNodeHash {
    type Error = FixedHashSizeError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let hash = FixedHash::try_from(value)?;
        Ok(Self(hash))
    }
}

impl AsRef<[u8]> for TreeNodeHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl From<TreeNodeHash> for Vec<u8> {
    fn from(s: TreeNodeHash) -> Self {
        s.as_bytes().to_vec()
    }
}

impl Hex for TreeNodeHash {
    fn from_hex(hex: &str) -> Result<Self, HexError>
    where Self: Sized {
        let hash = FixedHash::from_hex(hex)?;
        Ok(Self(hash))
    }

    fn to_hex(&self) -> String {
        self.0.to_hex()
    }
}

impl Display for TreeNodeHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}
