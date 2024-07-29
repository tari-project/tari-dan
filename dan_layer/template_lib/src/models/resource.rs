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
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::{BinaryTag, EntityId, KeyParseError, ObjectKey};
use crate::{newtype_struct_serde_impl, prelude::CONFIDENTIAL_TARI_RESOURCE_ADDRESS};

const TAG: u64 = BinaryTag::ResourceAddress.as_u64();

/// The unique identification of a resource in the Tari network
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ResourceAddress(#[cfg_attr(feature = "ts", ts(type = "string"))] BorTag<ObjectKey, TAG>);

impl ResourceAddress {
    pub const fn new(key: ObjectKey) -> Self {
        Self(BorTag::new(key))
    }

    pub fn as_object_key(&self) -> &ObjectKey {
        self.0.inner()
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_ref()
    }

    pub fn from_hex(hex: &str) -> Result<Self, KeyParseError> {
        let key = ObjectKey::from_hex(hex)?;
        Ok(Self::new(key))
    }

    pub fn as_entity_id(&self) -> EntityId {
        self.as_object_key().as_entity_id()
    }

    pub fn is_tari(&self) -> bool {
        *self == CONFIDENTIAL_TARI_RESOURCE_ADDRESS
    }
}

impl FromStr for ResourceAddress {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix("resource_").unwrap_or(s);
        Self::from_hex(s)
    }
}

impl<T: Into<ObjectKey>> From<T> for ResourceAddress {
    fn from(key: T) -> Self {
        Self::new(key.into())
    }
}

impl Display for ResourceAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "resource_{}", *self.0)
    }
}

impl AsRef<[u8]> for ResourceAddress {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl TryFrom<&[u8]> for ResourceAddress {
    type Error = KeyParseError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let key = ObjectKey::try_from(value)?;
        Ok(Self::new(key))
    }
}

newtype_struct_serde_impl!(ResourceAddress, BorTag<ObjectKey, TAG>);

#[cfg(test)]
mod tests {
    use super::*;

    mod hex_deser {
        use super::*;

        #[test]
        fn string_serialization_and_deserialization() {
            let resx_str = "resource_00000000000000000000000000000000000000000000000000000000";
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
