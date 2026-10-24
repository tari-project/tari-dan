//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use core::{fmt, str::FromStr};

use minicbor::{CborLen, Decode, Encode};
use tari_template_abi::rust::prelude::*;

use crate::{
    KeyParseError,
    hex::{bytes_from_hex, write_hex_fmt},
    max_bytes::MaxBytes,
};

const MAX_LEN: usize = 128;

/// An author-supplied reference to a template's off-chain metadata, carried by the publish instruction and stored on
/// the published template.
///
/// The protocol bounds the length and otherwise treats the bytes as opaque: it never parses or verifies them against
/// any data. Wallets conventionally put a multihash of the CBOR-encoded metadata here (see
/// `tari_ootle_template_metadata::MetadataHash`), and any format check belongs to them.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, CborLen)]
#[cbor(transparent)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct TemplateMetadataHash(MaxBytes<MAX_LEN>);

impl TemplateMetadataHash {
    pub const MAX_LEN: usize = MAX_LEN;

    /// Returns `None` if `bytes` is longer than [`Self::MAX_LEN`].
    pub fn from_bytes(bytes: impl Into<Box<[u8]>>) -> Option<Self> {
        MaxBytes::new_checked(bytes).map(Self)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl AsRef<[u8]> for TemplateMetadataHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl TryFrom<Vec<u8>> for TemplateMetadataHash {
    type Error = usize;

    /// Fails with the input length if it exceeds [`Self::MAX_LEN`].
    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let len = value.len();
        Self::from_bytes(value).ok_or(len)
    }
}

impl fmt::Display for TemplateMetadataHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex_fmt(f, &self.0)
    }
}

impl FromStr for TemplateMetadataHash {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = bytes_from_hex(s)?;
        Self::from_bytes(bytes).ok_or(KeyParseError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_arbitrary_bytes_up_to_the_limit() {
        assert!(TemplateMetadataHash::from_bytes(vec![]).is_some());
        assert!(TemplateMetadataHash::from_bytes(vec![0xff; MAX_LEN]).is_some());
        assert!(TemplateMetadataHash::from_bytes(vec![0xff; MAX_LEN + 1]).is_none());
    }

    #[test]
    fn cbor_is_a_byte_string() {
        let hash = TemplateMetadataHash::from_bytes(vec![1, 2, 3]).unwrap();
        let encoded = tari_bor::encode(&hash).unwrap();
        let value: tari_bor::Value = tari_bor::decode(&encoded).unwrap();
        assert_eq!(value, tari_bor::Value::Bytes(vec![1, 2, 3]));
        assert_eq!(tari_bor::decode::<TemplateMetadataHash>(&encoded).unwrap(), hash);
    }

    #[test]
    fn cbor_decode_rejects_oversized() {
        let encoded = tari_bor::encode(&tari_bor::Value::Bytes(vec![0; MAX_LEN + 1])).unwrap();
        assert!(tari_bor::decode::<TemplateMetadataHash>(&encoded).is_err());
    }

    #[test]
    fn hex_roundtrip() {
        let hash = TemplateMetadataHash::from_bytes(vec![0xab, 0x01]).unwrap();
        assert_eq!(hash.to_string(), "ab01");
        assert_eq!("ab01".parse::<TemplateMetadataHash>().unwrap(), hash);
    }
}
