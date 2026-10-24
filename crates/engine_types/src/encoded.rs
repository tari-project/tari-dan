//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{any::type_name, fmt, marker::PhantomData};

use minicbor::{CborLen, Decode, Decoder, Encode, Encoder, decode, encode};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tari_bor::BorError;

use crate::limits::MAX_CBOR_NESTING_DEPTH;

/// A `T` held in its CBOR encoding.
///
/// The bytes are kept exactly as they were received or encoded, so a value passed along as an `Encoded` reaches its
/// reader byte for byte: a verifier that decodes it hashes the value its producer committed. Nothing checks that the
/// bytes decode until [`Self::decode`] is called.
///
/// Serialised with serde as base64 in human-readable formats and as a byte string otherwise, and with minicbor as a
/// CBOR byte string.
pub struct Encoded<T> {
    bytes: Vec<u8>,
    _type: PhantomData<fn() -> T>,
}

impl<T> Encoded<T> {
    /// Holds `bytes` as the encoding of a `T`, unchecked.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            _type: PhantomData,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl<T: Encode<()>> Encoded<T> {
    pub fn encode(value: &T) -> Result<Self, BorError> {
        tari_bor::encode(value).map(Self::from_bytes)
    }
}

impl<T: for<'b> Decode<'b, ()>> Encoded<T> {
    /// The `T` these bytes encode. Every byte must belong to it, and it may nest at most [`MAX_CBOR_NESTING_DEPTH`]
    /// deep.
    pub fn decode(&self) -> Result<T, BorError> {
        tari_bor::decode_exact_with_max_depth(&self.bytes, MAX_CBOR_NESTING_DEPTH)
    }
}

impl<T> Clone for Encoded<T> {
    fn clone(&self) -> Self {
        Self::from_bytes(self.bytes.clone())
    }
}

impl<T> PartialEq for Encoded<T> {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl<T> Eq for Encoded<T> {}

impl<T> fmt::Debug for Encoded<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Encoded<{}>({} bytes)", type_name::<T>(), self.bytes.len())
    }
}

impl<T> Serialize for Encoded<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ootle_serde::base64::serialize(&self.bytes, serializer)
    }
}

impl<'de, T> Deserialize<'de> for Encoded<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ootle_serde::base64::deserialize::<_, Vec<u8>>(deserializer).map(Self::from_bytes)
    }
}

impl<C, T> Encode<C> for Encoded<T> {
    fn encode<W: encode::Write>(&self, e: &mut Encoder<W>, _ctx: &mut C) -> Result<(), encode::Error<W::Error>> {
        e.bytes(&self.bytes)?;
        Ok(())
    }
}

impl<'b, C, T> Decode<'b, C> for Encoded<T> {
    fn decode(d: &mut Decoder<'b>, _ctx: &mut C) -> Result<Self, decode::Error> {
        d.bytes().map(|bytes| Self::from_bytes(bytes.to_vec()))
    }
}

impl<C, T> CborLen<C> for Encoded<T> {
    fn cbor_len(&self, ctx: &mut C) -> usize {
        let bytes: &minicbor::bytes::ByteSlice = self.bytes.as_slice().into();
        bytes.cbor_len(ctx)
    }
}

#[cfg(test)]
mod tests {
    use tari_bor::Value;

    use super::*;
    use crate::{non_fungible::NonFungibleContainer, substate::SubstateValue};

    fn value() -> SubstateValue {
        SubstateValue::NonFungible(NonFungibleContainer::new(Value::Float(f64::NAN), Value::Null))
    }

    #[test]
    fn the_bytes_pass_through_serde_json_and_cbor_unchanged() {
        let encoded = Encoded::encode(&value()).unwrap();

        let json = serde_json::to_string(&encoded).unwrap();
        assert_eq!(serde_json::from_str::<Encoded<SubstateValue>>(&json).unwrap(), encoded);

        let cbor = tari_bor::encode(&encoded).unwrap();
        assert_eq!(cbor.len(), tari_bor::encoded_len(&encoded));
        assert_eq!(tari_bor::decode::<Encoded<SubstateValue>>(&cbor).unwrap(), encoded);
    }

    #[test]
    fn it_decodes_to_the_value_it_encodes() {
        let encoded = Encoded::encode(&value()).unwrap();
        assert_eq!(
            tari_bor::encode(&encoded.decode().unwrap()).unwrap(),
            encoded.as_bytes()
        );
    }

    #[test]
    fn trailing_bytes_do_not_decode() {
        let mut bytes = Encoded::encode(&value()).unwrap().into_bytes();
        bytes.push(0);
        assert!(Encoded::<SubstateValue>::from_bytes(bytes).decode().is_err());
    }
}
