//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//  Copyright 2022. The Tari Project
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
    ops::{Deref, DerefMut},
};

const ZERO_HASH: [u8; TreeHash::byte_size()] = [0u8; TreeHash::byte_size()];

#[derive(thiserror::Error, Debug)]
#[error("Invalid size")]
pub struct TreeHashSizeError;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Hash)]
pub struct TreeHash([u8; TreeHash::byte_size()]);

impl TreeHash {
    pub const fn new(hash: [u8; TreeHash::byte_size()]) -> Self {
        Self(hash)
    }

    pub const fn byte_size() -> usize {
        32
    }

    pub const fn zero() -> Self {
        Self(ZERO_HASH)
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl From<[u8; TreeHash::byte_size()]> for TreeHash {
    fn from(hash: [u8; TreeHash::byte_size()]) -> Self {
        Self(hash)
    }
}

impl TryFrom<Vec<u8>> for TreeHash {
    type Error = TreeHashSizeError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        TryFrom::try_from(value.as_slice())
    }
}

impl TryFrom<&[u8]> for TreeHash {
    type Error = TreeHashSizeError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        if bytes.len() != TreeHash::byte_size() {
            return Err(TreeHashSizeError);
        }

        let mut buf = [0u8; TreeHash::byte_size()];
        buf.copy_from_slice(bytes);
        Ok(Self(buf))
    }
}

impl PartialEq<[u8]> for TreeHash {
    fn eq(&self, other: &[u8]) -> bool {
        self.0[..].eq(other)
    }
}

impl PartialEq<TreeHash> for [u8] {
    fn eq(&self, other: &TreeHash) -> bool {
        self[..].eq(&other.0)
    }
}

impl PartialEq<Vec<u8>> for TreeHash {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self == other.as_slice()
    }
}
impl PartialEq<TreeHash> for Vec<u8> {
    fn eq(&self, other: &TreeHash) -> bool {
        self == other.as_slice()
    }
}

impl AsRef<[u8]> for TreeHash {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Deref for TreeHash {
    type Target = [u8; TreeHash::byte_size()];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TreeHash {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Display for TreeHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        hex::encode(&self.0).fmt(f)
    }
}
