//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, io};

use multihash::Multihash;
use serde::{Deserialize, Serialize, de};
use sha2::{Digest, Sha256, digest::Update};

/// Multihash function code for SHA-256 (IPFS CIDv1 default).
const SHA2_256_CODE: u64 = 0x12;

/// Domain separation label for template metadata hashing.
const DOMAIN_LABEL: &str = "com.tari.ootle.TemplateMetadata";

/// Maximum size of a metadata multihash digest in bytes.
/// This allows SHA-512 (64 bytes) and is also the IPFS CID default size.
pub const MAX_DIGEST_SIZE: usize = 64;

/// Multihash-encoded hash of template metadata CBOR.
///
/// Stack-allocated via [`multihash::Multihash`] — no heap allocation.
/// Maximum digest size is [`MAX_DIGEST_SIZE`] bytes.
///
/// Default hash function: SHA-256 (code 0x12), compatible with IPFS CIDv1.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct MetadataHash(Multihash<MAX_DIGEST_SIZE>);

impl MetadataHash {
    /// Create a MetadataHash from a raw multihash byte slice.
    /// Returns `None` if the bytes are not a valid multihash or exceed the digest size limit.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        Multihash::from_bytes(bytes).ok().map(Self)
    }

    /// Verify that this hash matches the given data by re-hashing with domain-separated SHA-256.
    /// Returns an error if the hash function is unsupported.
    pub fn verify(&self, pre_image: &[u8]) -> Result<bool, MetadataHashError> {
        match self.0.code() {
            SHA2_256_CODE => {
                let mut writer = MetadataHashWriter::new();
                io::Write::write_all(&mut writer, pre_image).expect("MetadataHashWriter is infallible");
                let expected = writer.finalize();
                Ok(self == &expected)
            },
            code => Err(MetadataHashError::UnsupportedHashFunction(code)),
        }
    }

    /// Return the raw multihash-encoded bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.to_bytes()
    }

    /// Return the multihash bytes as a hex string.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0.to_bytes())
    }

    /// Parse a MetadataHash from a hex string.
    pub fn from_hex(hex: &str) -> Result<Self, MetadataHashError> {
        let bytes = hex::decode(hex).map_err(|_| MetadataHashError::InvalidHex)?;
        Self::from_bytes(&bytes).ok_or(MetadataHashError::InvalidMultihash {
            details: "Invalid Multihash bytes from hex".to_string(),
        })
    }

    /// Return the inner `Multihash`.
    pub fn into_inner(self) -> Multihash<MAX_DIGEST_SIZE> {
        self.0
    }

    /// Return a reference to the inner `Multihash`.
    pub fn as_multihash(&self) -> &Multihash<MAX_DIGEST_SIZE> {
        &self.0
    }
}

impl From<Multihash<MAX_DIGEST_SIZE>> for MetadataHash {
    fn from(mh: Multihash<MAX_DIGEST_SIZE>) -> Self {
        Self(mh)
    }
}

impl fmt::Debug for MetadataHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MetadataHash({})", self.to_hex())
    }
}

impl fmt::Display for MetadataHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Serialize for MetadataHash {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for MetadataHash {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let hex = String::deserialize(deserializer)?;
        MetadataHash::from_hex(&hex).map_err(de::Error::custom)
    }
}

#[cfg(feature = "borsh")]
impl borsh::BorshSerialize for MetadataHash {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        // Multihash write already represents the hash canonically
        self.0.write(writer).map_err(io::Error::other)?;
        Ok(())
    }
}

impl<C> minicbor::Encode<C> for MetadataHash {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        _ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        let bytes = self.to_bytes();
        e.bytes(&bytes)?;
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for MetadataHash {
    fn decode(d: &mut minicbor::Decoder<'b>, _ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        let bytes = d.bytes()?;
        MetadataHash::from_bytes(bytes)
            .ok_or_else(|| minicbor::decode::Error::message("invalid MetadataHash multihash bytes"))
    }
}

impl<C> minicbor::CborLen<C> for MetadataHash {
    fn cbor_len(&self, ctx: &mut C) -> usize {
        let bytes = self.to_bytes();
        minicbor::bytes::cbor_len(bytes.as_slice(), ctx)
    }
}

/// A domain-separated SHA-256 hasher that implements [`std::io::Write`].
///
/// CBOR (or any data) can be written directly into this without intermediate allocation.
/// Call [`finalize`](MetadataHashWriter::finalize) to produce the [`MetadataHash`].
///
/// The hash is computed as: `SHA-256("com.tari.ootle.TemplateMetadata" || data)`.
pub struct MetadataHashWriter {
    hasher: Sha256,
}

impl MetadataHashWriter {
    /// Create a new writer with the domain separation label already fed into the hasher.
    pub fn new() -> Self {
        Self {
            hasher: Sha256::new().chain(DOMAIN_LABEL),
        }
    }

    /// Finalize the hash and return the [`MetadataHash`].
    pub fn finalize(self) -> MetadataHash {
        let digest = self.hasher.finalize();
        MetadataHash(Multihash::wrap(SHA2_256_CODE, &digest).expect("SHA-256 digest fits in 64 bytes"))
    }
}

impl Default for MetadataHashWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl io::Write for MetadataHashWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Update::update(&mut self.hasher, buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MetadataHashError {
    #[error("Unsupported hash function code: 0x{0:02x}")]
    UnsupportedHashFunction(u64),
    #[error("Invalid hex string")]
    InvalidHex,
    #[error("Invalid multihash encoding: {details}")]
    InvalidMultihash { details: String },
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn hash_data(data: &[u8]) -> MetadataHash {
        let mut writer = MetadataHashWriter::new();
        writer.write_all(data).unwrap();
        writer.finalize()
    }

    #[test]
    fn writer_produces_valid_multihash() {
        let hash = hash_data(b"hello world");

        // SHA-256 multihash: code=0x12, size=0x20 (32 bytes), then 32 bytes digest
        let bytes = hash.to_bytes();
        assert_eq!(bytes[0], 0x12);
        assert_eq!(bytes[1], 32);
        assert_eq!(bytes.len(), 34);
    }

    #[test]
    fn writer_verify_roundtrip() {
        let data = b"hello world";
        let hash = hash_data(data);

        assert!(hash.verify(data).unwrap());
        assert!(!hash.verify(b"different data").unwrap());
    }

    #[test]
    fn incremental_write_matches_single_write() {
        let data = b"hello world, this is a longer message for testing";

        let hash_single = hash_data(data);

        let mut writer = MetadataHashWriter::new();
        writer.write_all(&data[..5]).unwrap();
        writer.write_all(&data[5..]).unwrap();
        let hash_incremental = writer.finalize();

        assert_eq!(hash_single, hash_incremental);
    }

    #[test]
    fn stack_allocated() {
        let hash = hash_data(b"test");
        assert!(std::mem::size_of_val(&hash) <= std::mem::size_of::<Multihash<MAX_DIGEST_SIZE>>());
    }

    #[test]
    fn from_bytes_rejects_invalid() {
        assert!(MetadataHash::from_bytes(&[0x12]).is_none());
        let hash = hash_data(b"test");
        let bytes = hash.to_bytes();
        assert!(MetadataHash::from_bytes(&bytes).is_some());
    }

    #[test]
    fn hex_roundtrip() {
        let hash = hash_data(b"test");
        let hex = hash.to_hex();
        let parsed = MetadataHash::from_hex(&hex).unwrap();
        assert_eq!(hash, parsed);
    }

    #[test]
    fn json_serde_roundtrip() {
        let hash = hash_data(b"test");
        let json = serde_json::to_string(&hash).unwrap();
        let parsed: MetadataHash = serde_json::from_str(&json).unwrap();
        assert_eq!(hash, parsed);
    }

    #[test]
    fn cbor_serde_roundtrip() {
        let hash = hash_data(b"test");
        let cbor = tari_bor::encode(&hash).unwrap();
        let parsed: MetadataHash = tari_bor::decode(&cbor).unwrap();
        assert_eq!(hash, parsed);
    }

    #[test]
    fn cbor_len_matches_the_encoding() {
        let hash = hash_data(b"test");
        assert_eq!(minicbor::len(&hash), tari_bor::encode(&hash).unwrap().len());
    }

    #[test]
    fn to_bytes_roundtrip() {
        let hash = hash_data(b"test");
        let bytes = hash.to_bytes();
        let parsed = MetadataHash::from_bytes(&bytes).unwrap();
        assert_eq!(hash, parsed);
    }
}
