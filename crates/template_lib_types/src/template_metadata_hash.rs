//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use core::{fmt, str::FromStr};

use minicbor::{CborLen, Encode};
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
/// The protocol bounds the length and stores the bytes opaquely. Wallets conventionally put a multihash of the
/// CBOR-encoded metadata here (see `tari_ootle_template_metadata::MetadataHash`), and any format check belongs to them.
///
/// The value is never empty, so `None` is the only encoding of "no metadata reference".
#[derive(Debug, Clone, PartialEq, Eq, Encode, CborLen)]
#[cbor(transparent)]
#[cfg_attr(feature = "serde", derive(serde::Serialize), serde(transparent))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct TemplateMetadataHash(MaxBytes<MAX_LEN>);

impl TemplateMetadataHash {
    pub const MAX_LEN: usize = MAX_LEN;

    /// Returns `None` if `bytes` is empty or longer than [`Self::MAX_LEN`].
    pub fn from_bytes(bytes: impl Into<Box<[u8]>>) -> Option<Self> {
        Self::from_max_bytes(MaxBytes::new_checked(bytes)?)
    }

    fn from_max_bytes(bytes: MaxBytes<MAX_LEN>) -> Option<Self> {
        if bytes.is_empty() { None } else { Some(Self(bytes)) }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for TemplateMetadataHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl TryFrom<Vec<u8>> for TemplateMetadataHash {
    type Error = usize;

    /// Fails with the input length if it is empty or exceeds [`Self::MAX_LEN`].
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

impl<'b, C> minicbor::Decode<'b, C> for TemplateMetadataHash {
    fn decode(d: &mut minicbor::Decoder<'b>, ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        let bytes = MaxBytes::decode(d, ctx)?;
        Self::from_max_bytes(bytes).ok_or_else(|| minicbor::decode::Error::message("template metadata hash is empty"))
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for TemplateMetadataHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        let bytes = MaxBytes::deserialize(deserializer)?;
        Self::from_max_bytes(bytes).ok_or_else(|| serde::de::Error::custom("template metadata hash is empty"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_arbitrary_bytes_within_the_bounds() {
        assert!(TemplateMetadataHash::from_bytes(vec![]).is_none());
        assert!(TemplateMetadataHash::from_bytes(vec![0x00]).is_some());
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
    fn cbor_decode_rejects_out_of_bounds_lengths() {
        for len in [0, MAX_LEN + 1] {
            let encoded = tari_bor::encode(&tari_bor::Value::Bytes(vec![0; len])).unwrap();
            assert!(tari_bor::decode::<TemplateMetadataHash>(&encoded).is_err(), "len {len}");
        }
    }

    #[test]
    fn hex_roundtrip() {
        let hash = TemplateMetadataHash::from_bytes(vec![0xab, 0x01]).unwrap();
        assert_eq!(hash.to_string(), "ab01");
        assert_eq!("ab01".parse::<TemplateMetadataHash>().unwrap(), hash);
        assert!("".parse::<TemplateMetadataHash>().is_err());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn json_is_hex_and_rejects_empty() {
        let hash = TemplateMetadataHash::from_bytes(vec![0xab, 0x01]).unwrap();
        assert_eq!(serde_json::to_string(&hash).unwrap(), r#""ab01""#);
        assert_eq!(serde_json::from_str::<TemplateMetadataHash>(r#""ab01""#).unwrap(), hash);
        assert!(serde_json::from_str::<TemplateMetadataHash>(r#""""#).is_err());
    }

    /// The signing preimage of a publish instruction and the published template's substate hash both embed this
    /// encoding: a little-endian `u32` length, then the bytes.
    #[cfg(feature = "borsh")]
    #[test]
    fn borsh_is_length_prefixed() {
        let hash = TemplateMetadataHash::from_bytes(vec![1, 2, 3]).unwrap();
        assert_eq!(borsh::to_vec(&hash).unwrap(), [3, 0, 0, 0, 1, 2, 3]);
    }
}
