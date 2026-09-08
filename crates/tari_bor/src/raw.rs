//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[cfg(not(feature = "std"))]
use alloc::boxed::Box;

use minicbor::{
    CborLen,
    Decoder,
    Encoder,
    decode,
    encode::{self, Write},
};

use crate::{BorError, Value, decode_exact, encode as encode_to_vec};

/// One CBOR item held as its own encoding.
///
/// Encoding writes the bytes back verbatim and decoding captures them without interpreting the
/// item, so encoding a `RawCbor` yields the same bytes as encoding the value it was built from.
///
/// The bytes are always exactly one complete item — [`Self::from_encodable`] and the [`Decode`]
/// impl are the only ways to build one, and both guarantee it. Nothing mutates them afterwards.
///
/// [`Decode`]: minicbor::Decode
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCbor(Box<[u8]>);

impl RawCbor {
    /// Encodes `value` and keeps the result.
    pub fn from_encodable<T: minicbor::Encode<()> + ?Sized>(value: &T) -> Result<Self, BorError> {
        Ok(Self(encode_to_vec(value)?.into_boxed_slice()))
    }

    /// Encodes a dynamic value tree and keeps the result.
    pub fn from_value(value: &Value) -> Result<Self, BorError> {
        Self::from_encodable(value)
    }

    /// Decodes the item into `T`.
    pub fn decode<T: for<'b> minicbor::Decode<'b, ()>>(&self) -> Result<T, BorError> {
        decode_exact(&self.0)
    }

    /// Decodes the item into a dynamic value tree.
    pub fn to_value(&self) -> Result<Value, BorError> {
        self.decode()
    }

    /// The item's CBOR encoding.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl<C> minicbor::Encode<C> for RawCbor {
    fn encode<W: Write>(&self, e: &mut Encoder<W>, _ctx: &mut C) -> Result<(), encode::Error<W::Error>> {
        e.writer_mut().write_all(&self.0).map_err(encode::Error::write)
    }
}

impl<'b, C> minicbor::Decode<'b, C> for RawCbor {
    fn decode(d: &mut Decoder<'b>, _ctx: &mut C) -> Result<Self, decode::Error> {
        let start = d.position();
        // Iterative in minicbor, with its own heap stack, so nesting depth in untrusted input
        // cannot drive this into a stack overflow the way a recursive walk would.
        d.skip()?;
        let end = d.position();
        let bytes = d
            .input()
            .get(start..end)
            .ok_or_else(|| decode::Error::message("RawCbor: skipped past the end of the input"))?;
        Ok(Self(Box::from(bytes)))
    }
}

impl<C> CborLen<C> for RawCbor {
    fn cbor_len(&self, _ctx: &mut C) -> usize {
        self.0.len()
    }
}

/// Serde sees the item, not its encoding: a `RawCbor` serialises as the [`Value`] it holds, so a
/// field carrying one has the same JSON as a field carrying the value itself. Serde is a JSON-style
/// path only — CBOR never travels through it — so the conversion is off the encoding path.
///
/// Unlike the CBOR path this is not byte-preserving in both directions. Deserialising rebuilds the
/// encoding from the value, so an item that arrived non-canonically comes back canonical, and
/// serialising fails for an item `Value` cannot represent.
#[cfg(feature = "serde")]
impl serde::Serialize for RawCbor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_value()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for RawCbor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "std"))]
    use alloc::string::ToString;

    use super::*;
    use crate::{encode, to_value};

    /// A payload routed through a `Value` tree and back encodes to the payload's own bytes.
    #[test]
    fn value_round_trip_is_byte_identical() {
        fn check<T: minicbor::Encode<()> + ?Sized>(value: &T) {
            let direct = encode(value).unwrap();
            let via_value = encode(&to_value(value).unwrap()).unwrap();
            assert_eq!(direct, via_value);
            assert_eq!(RawCbor::from_encodable(value).unwrap().as_bytes(), direct.as_slice());
        }

        check(&());
        check(&u64::MAX);
        check(&i64::MIN);
        check(&0u8);
        check(&true);
        check("a string");
        check(&vec![1u64, 2, 3]);
        check(&(1u8, "two", vec![3u8]));
        check(&Some(vec![vec![1u8, 2], vec![3]]));
        check(&None::<u32>);
    }

    /// The limit of the substitution: `Value` has no `f32`, so a float widens to `f64` passing
    /// through one. Every type crossing the ABI must therefore be float-free, and this fails if
    /// that widening ever stops happening.
    #[test]
    fn a_float_is_the_one_shape_a_value_does_not_reproduce() {
        let direct = encode(&1.5f32).unwrap();
        let via_value = encode(&to_value(&1.5f32).unwrap()).unwrap();
        assert_ne!(direct, via_value);
        assert_eq!(direct.len(), 5, "f32 header plus four bytes");
        assert_eq!(via_value.len(), 9, "f64 header plus eight bytes");
    }

    /// An integer outside CBOR's own range has no encoding, so it never reaches a `RawCbor` — which
    /// is why the JSON round trip above does not cover it.
    #[test]
    fn an_integer_beyond_cbor_range_cannot_be_held() {
        assert!(RawCbor::from_value(&Value::Integer(i128::from(u64::MAX) + 1)).is_err());
        assert!(RawCbor::from_value(&Value::Integer(i128::from(i64::MIN) - 1)).is_err());
    }

    #[test]
    fn decode_captures_exactly_one_item() {
        // Two items back to back: decoding the first must stop at its own end.
        let mut buf = encode(&vec![1u64, 2, 3]).unwrap();
        let second = encode(&"tail").unwrap();
        buf.extend_from_slice(&second);

        let mut decoder = Decoder::new(&buf);
        let first: RawCbor = decoder.decode().unwrap();
        assert_eq!(first.as_bytes(), encode(&vec![1u64, 2, 3]).unwrap().as_slice());
        let rest: RawCbor = decoder.decode().unwrap();
        assert_eq!(rest.as_bytes(), second.as_slice());
    }

    #[test]
    fn encode_writes_the_item_back_verbatim() {
        let raw = RawCbor::from_encodable(&vec!["a", "b"]).unwrap();
        assert_eq!(encode(&raw).unwrap(), raw.as_bytes());
        assert_eq!(raw.decode::<Vec<String>>().unwrap(), vec![
            "a".to_string(),
            "b".to_string()
        ]);
    }

    /// A `RawCbor` field and a `Value` field holding the same item have the same JSON.
    #[cfg(feature = "serde")]
    #[test]
    fn json_matches_the_value_it_holds() {
        for value in [
            Value::Null,
            Value::Bool(true),
            Value::Integer(-7),
            Value::Integer(i128::from(u64::MAX)),
            Value::Bytes(vec![0xAB, 0x12]),
            Value::Text("text".to_string()),
            Value::Array(vec![Value::Integer(1), Value::Text("two".to_string())]),
            Value::Map(vec![(Value::Text("k".to_string()), Value::Integer(1))]),
            // The variants `value_serde` routes through its `@cbor` sentinel rather than a natural
            // JSON shape, where a round trip has the furthest to fall.
            Value::Map(vec![(Value::Integer(1), Value::Text("non-text key".to_string()))]),
            Value::Tag(42, Box::new(Value::Integer(7))),
            Value::Tag(42, Box::new(Value::Bytes(vec![1, 2, 3]))),
        ] {
            let raw = RawCbor::from_value(&value).unwrap();
            assert_eq!(
                serde_json::to_string(&raw).unwrap(),
                serde_json::to_string(&value).unwrap()
            );
            let back: RawCbor = serde_json::from_str(&serde_json::to_string(&raw).unwrap()).unwrap();
            assert_eq!(back, raw);
        }
    }

    #[test]
    fn cbor_len_matches_the_encoding() {
        let raw = RawCbor::from_encodable(&vec![0u8; 300]).unwrap();
        assert_eq!(crate::encoded_len(&raw), raw.as_bytes().len());
    }
}
