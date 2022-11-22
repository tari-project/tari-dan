//  Copyright 2022, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    convert::TryFrom,
    fmt::{Display, Formatter},
    io::{self, Write},
};

use borsh::BorshSerialize;
use digest::{consts::U32, generic_array};
use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, FixedHashSizeError};
use tari_dan_common_types::serde_with;
use tari_utilities::hex::{Hex, HexError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct TreeNodeHash(#[serde(with = "serde_with::hex")] FixedHash);

// TODO: remove this implementation once the Borsh is on FixedHash
impl BorshSerialize for TreeNodeHash {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        BorshSerialize::serialize(&self.as_bytes(), writer)
    }
}

impl TreeNodeHash {
    pub fn zero() -> Self {
        Self(FixedHash::zero())
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
