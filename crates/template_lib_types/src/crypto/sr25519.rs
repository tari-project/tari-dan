//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Encode};
use tari_template_abi::rust::{
    fmt,
    fmt::{Display, Formatter},
    str::FromStr,
};

use crate::{KeyParseError, crypto::InvalidByteLengthError, hex::fixed_bytes_from_hex};

/// A 64-byte sr25519 signature in its standard encoding: the public nonce `R`, then the scalar `s`
/// with the marker bit set in its last byte.
///
/// Construction and decoding accept any 64 bytes; verification rejects an encoding that is not a
/// well-formed signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode, CborLen)]
#[cbor(transparent)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Sr25519SignatureBytes(
    #[cfg_attr(feature = "serde", serde(with = "crate::serde_helpers::fixed_hex"))]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cbor(with = "minicbor::bytes")]
    [u8; Sr25519SignatureBytes::LENGTH],
);

impl Sr25519SignatureBytes {
    pub const LENGTH: usize = 64;

    pub const fn from_array(bytes: [u8; Self::LENGTH]) -> Self {
        Self(bytes)
    }

    pub const fn from_bytes(bytes: &[u8]) -> Result<Self, InvalidByteLengthError> {
        if bytes.len() != Self::LENGTH {
            return Err(InvalidByteLengthError {
                size: bytes.len(),
                expected: Self::LENGTH,
            });
        }
        let mut signature = [0u8; Self::LENGTH];
        signature.copy_from_slice(bytes);
        Ok(Self(signature))
    }

    /// All-zero bytes, a placeholder that never verifies.
    pub const fn zero() -> Self {
        Self([0u8; Self::LENGTH])
    }

    pub const fn as_array(&self) -> &[u8; Self::LENGTH] {
        &self.0
    }

    pub const fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<[u8; Sr25519SignatureBytes::LENGTH]> for Sr25519SignatureBytes {
    fn from(bytes: [u8; Sr25519SignatureBytes::LENGTH]) -> Self {
        Self(bytes)
    }
}

impl TryFrom<&[u8]> for Sr25519SignatureBytes {
    type Error = InvalidByteLengthError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::from_bytes(value)
    }
}

impl AsRef<[u8]> for Sr25519SignatureBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl FromStr for Sr25519SignatureBytes {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(fixed_bytes_from_hex(s)?))
    }
}

impl Display for Sr25519SignatureBytes {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_as_64_bytes() {
        let signature = Sr25519SignatureBytes::from_array([7u8; 64]);
        let cbor = minicbor::to_vec(signature).unwrap();
        assert_eq!(minicbor::decode::<Sr25519SignatureBytes>(&cbor).unwrap(), signature);
        assert_eq!(
            signature.to_string().parse::<Sr25519SignatureBytes>().unwrap(),
            signature
        );
    }

    #[cfg(feature = "borsh")]
    #[test]
    fn borsh_is_the_raw_64_bytes() {
        let signature = Sr25519SignatureBytes::from_array([7u8; 64]);
        assert_eq!(borsh::to_vec(&signature).unwrap(), vec![7u8; 64]);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_is_a_hex_string() {
        let signature = Sr25519SignatureBytes::from_array([7u8; 64]);
        let json = serde_json::to_string(&signature).unwrap();
        assert_eq!(json, format!("\"{}\"", "07".repeat(64)));
        assert_eq!(serde_json::from_str::<Sr25519SignatureBytes>(&json).unwrap(), signature);
    }
}
