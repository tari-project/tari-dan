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

use tari_bor::BorTag;
use tari_template_abi::rust::{
    fmt,
    fmt::{Display, Formatter},
    str::FromStr,
};

use super::BinaryTag;
use crate::{hash::HashParseError, newtype_struct_serde_impl, Hash};

const TAG: u64 = BinaryTag::ResourceAddress.as_u64();

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceAddress(BorTag<Hash, TAG>);

impl ResourceAddress {
    pub const fn new(address: Hash) -> Self {
        Self(BorTag::new(address))
    }

    pub fn hash(&self) -> &Hash {
        &self.0
    }

    pub fn from_hex(hex: &str) -> Result<Self, HashParseError> {
        let hash = Hash::from_hex(hex)?;
        Ok(Self::new(hash))
    }
}

impl FromStr for ResourceAddress {
    type Err = HashParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = if let Some(stripped) = s.strip_prefix("resource_") {
            stripped
        } else {
            s
        };
        let hash = Hash::from_hex(s)?;
        Ok(Self::new(hash))
    }
}

impl<T: Into<Hash>> From<T> for ResourceAddress {
    fn from(address: T) -> Self {
        Self::new(address.into())
    }
}

impl Display for ResourceAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "resource_{}", *self.0)
    }
}

impl AsRef<[u8]> for ResourceAddress {
    fn as_ref(&self) -> &[u8] {
        self.hash()
    }
}

impl TryFrom<Vec<u8>> for ResourceAddress {
    type Error = HashParseError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let hash = Hash::try_from(value)?;
        Ok(Self::new(hash))
    }
}

newtype_struct_serde_impl!(ResourceAddress, BorTag<Hash, TAG>);

#[cfg(test)]
mod tests {
    use super::*;

    mod hex_deser {
        use super::*;

        #[test]
        fn string_serialization_and_deserialization() {
            let resx_str = "resource_0000000000000000000000000000000000000000000000000000000000000000";
            let resource = ResourceAddress::from_str(resx_str).unwrap();
            let json = serde_json::to_string_pretty(&resource).unwrap();
            assert_eq!(json.trim_matches('"'), resx_str);

            // Deserialize from JSON
            let r = serde_json::from_str::<ResourceAddress>(&json).unwrap();
            assert_eq!(r, resource);

            // Check that CBOR does not include the string
            let cbor = tari_bor::encode(&resource).unwrap();
            assert!(
                !cbor.windows(resx_str.len()).any(|window| window == resx_str.as_bytes()),
                "CBOR is serializing a string"
            );

            // Deserialize from CBOR
            let r = tari_bor::decode::<ResourceAddress>(&cbor).unwrap();
            assert_eq!(r, resource);
        }
    }
}
