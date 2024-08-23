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

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, Bytes};
use tari_template_abi::rust::{
    fmt,
    fmt::{Display, Formatter},
    ops::{Deref, DerefMut},
    str::FromStr,
};

/// Representation of a 32-byte hash value
#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Hash(#[serde_as(as = "Bytes")] [u8; Self::LENGTH]);

impl Hash {
    pub const LENGTH: usize = 32;

    pub const fn from_array(bytes: [u8; Self::LENGTH]) -> Self {
        Self(bytes)
    }

    pub const fn into_array(self) -> [u8; Self::LENGTH] {
        self.0
    }

    pub fn from_hex(s: &str) -> Result<Self, HashParseError> {
        if s.len() != Self::LENGTH * 2 {
            return Err(HashParseError);
        }

        let mut hash = [0u8; Self::LENGTH];
        for (i, h) in hash.iter_mut().enumerate() {
            *h = u8::from_str_radix(&s[2 * i..2 * (i + 1)], 16).map_err(|_| HashParseError)?;
        }
        Ok(Hash(hash))
    }

    pub fn write_hex_fmt<W: fmt::Write>(&self, writer: &mut W) -> fmt::Result {
        for b in self.0 {
            write!(writer, "{:02x?}", b)?;
        }
        Ok(())
    }

    pub fn try_from_vec(data: Vec<u8>) -> Result<Self, HashParseError> {
        Self::try_from(data.as_slice())
    }

    /// Returns the leading `N` bytes of the hash
    ///
    /// # Panics
    ///
    /// Panics if `N` is greater than Self::LENGTH (32)
    pub fn leading_bytes<const N: usize>(&self) -> [u8; N] {
        self.0[..N].try_into().unwrap()
    }

    /// Returns the trailing `N` bytes of the hash
    ///
    /// # Panics
    ///
    /// Panics if `N` is greater than Self::LENGTH (32)
    pub fn trailing_bytes<const N: usize>(&self) -> [u8; N] {
        self.0[(Self::LENGTH - N)..Self::LENGTH].try_into().unwrap()
    }
}

impl AsRef<[u8]> for Hash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl From<[u8; Self::LENGTH]> for Hash {
    fn from(hash: [u8; Self::LENGTH]) -> Self {
        Self::from_array(hash)
    }
}

impl FromStr for Hash {
    type Err = HashParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Hash::from_hex(s)
    }
}

impl TryFrom<&[u8]> for Hash {
    type Error = HashParseError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.len() != Self::LENGTH {
            return Err(HashParseError);
        }
        let mut hash = [0u8; Self::LENGTH];
        hash.copy_from_slice(value);
        Ok(Hash::from_array(hash))
    }
}

impl TryFrom<Vec<u8>> for Hash {
    type Error = HashParseError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Hash::try_from(value.as_slice())
    }
}

impl Deref for Hash {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Hash {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Display for Hash {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for x in self.0 {
            write!(f, "{:02x?}", x)?;
        }
        Ok(())
    }
}

/// Representation of a hash parsing error
#[derive(Debug)]
pub struct HashParseError;

#[cfg(feature = "std")]
impl std::error::Error for HashParseError {}

impl Display for HashParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to parse hash")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_deserialize() {
        let hash = Hash::default();
        let mut buf = Vec::new();
        tari_bor::encode_into(&hash, &mut buf).unwrap();
        let hash2 = tari_bor::decode(&buf).unwrap();
        assert_eq!(hash, hash2);
    }
}
