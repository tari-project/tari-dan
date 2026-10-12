//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
//
//! Serde `Serialize` / `Deserialize` for [`Value`].
//!
//! Used for JSON and for the [`crate::serde_codec`] CBOR route, which stores serde types holding a
//! `Value`. Consensus encoding goes through minicbor's `Encode`/`Decode` and never through serde.
//!
//! ## Representation
//!
//! Values that map cleanly onto JSON are serialised directly:
//!
//! | Variant | JSON |
//! |---------|------|
//! | `Null` | `null` |
//! | `Bool(b)` | bool |
//! | `Integer(i)` (fits in i64) | number |
//! | `Integer(i)` (fits in u64 but not i64) | number |
//! | `Float(f)` | number |
//! | `Text(s)` | string |
//! | `Array(a)` | array |
//! | `Map(m)` where every key is `Text` and none is reserved | object |
//!
//! Variants without a natural JSON shape are emitted using a sentinel object keyed on
//! `"@cbor"`:
//!
//! | Variant | JSON |
//! |---------|------|
//! | `Bytes(b)` | `{ "@cbor": "bytes", "hex": "ab12.." }` |
//! | `Integer(i)` (outside i64/u64 range) | `{ "@cbor": "int", "value": "12345" }` |
//! | `Map(m)` with a non-text or reserved key | `{ "@cbor": "map", "entries": [[k, v], ...] }` |
//! | `Tag(t, v)` | `{ "@cbor": "tag", "tag": N, "value": v }` |
//!
//! The reserved keys are `"@cbor"` and `"$serde_json::private::Number"`, the two first keys the
//! decoder reads as a special form.
//!
//! Deserialisation accepts either the natural form (where unambiguous) or the sentinel
//! form. Hex strings without the sentinel envelope are NOT decoded as bytes — only the
//! sentinel form preserves byte semantics through a round-trip.

use core::fmt;

use serde::{
    Deserialize,
    Deserializer,
    Serialize,
    Serializer,
    de::{self, MapAccess, SeqAccess, Visitor},
    ser::{SerializeMap, SerializeSeq},
};

use crate::Value;

const SENTINEL_KEY: &str = "@cbor";
const SENTINEL_BYTES: &str = "bytes";
const SENTINEL_INT: &str = "int";
const SENTINEL_MAP: &str = "map";
const SENTINEL_TAG: &str = "tag";

/// With `serde_json/arbitrary_precision` enabled, numbers are encoded as a one-entry map
/// keyed on this sentinel string and a stringified value. Any workspace member can light
/// this up; we must accept it on decode regardless of who turned the feature on.
const JSON_NUMBER_SENTINEL: &str = "$serde_json::private::Number";

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            // `serialize_unit` is format-defined: `serde_codec` writes `()` as an empty array.
            Value::Null => s.serialize_none(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Integer(i) => {
                if let Ok(n) = i64::try_from(*i) {
                    s.serialize_i64(n)
                } else if let Ok(n) = u64::try_from(*i) {
                    s.serialize_u64(n)
                } else {
                    let mut m = s.serialize_map(Some(2))?;
                    m.serialize_entry(SENTINEL_KEY, SENTINEL_INT)?;
                    m.serialize_entry("value", &i.to_string())?;
                    m.end()
                }
            },
            Value::Float(f) => s.serialize_f64(*f),
            Value::Bytes(b) => {
                let mut m = s.serialize_map(Some(2))?;
                m.serialize_entry(SENTINEL_KEY, SENTINEL_BYTES)?;
                m.serialize_entry("hex", &encode_hex(b))?;
                m.end()
            },
            Value::Text(t) => s.serialize_str(t),
            Value::Array(arr) => {
                let mut seq = s.serialize_seq(Some(arr.len()))?;
                for v in arr {
                    seq.serialize_element(v)?;
                }
                seq.end()
            },
            Value::Map(m) => {
                if m.iter().all(|(k, _)| is_native_object_key(k)) {
                    let mut map = s.serialize_map(Some(m.len()))?;
                    for (k, v) in m {
                        if let Value::Text(k_str) = k {
                            map.serialize_entry(k_str, v)?;
                        }
                    }
                    map.end()
                } else {
                    let mut map = s.serialize_map(Some(2))?;
                    map.serialize_entry(SENTINEL_KEY, SENTINEL_MAP)?;
                    map.serialize_entry("entries", m)?;
                    map.end()
                }
            },
            Value::Tag(t, v) => {
                let mut m = s.serialize_map(Some(3))?;
                m.serialize_entry(SENTINEL_KEY, SENTINEL_TAG)?;
                m.serialize_entry("tag", t)?;
                m.serialize_entry("value", v.as_ref())?;
                m.end()
            },
        }
    }
}

/// The decoder reads an object whose first key is reserved as a special form, and any key may come
/// first once the JSON passes through a key-sorting map such as `serde_json::Value`. A map holding a
/// reserved key anywhere therefore goes through the `@cbor:map` envelope.
fn is_native_object_key(key: &Value) -> bool {
    matches!(key, Value::Text(k) if k != SENTINEL_KEY && k != JSON_NUMBER_SENTINEL)
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a CBOR value (JSON-compatible representation)")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(ValueVisitor)
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Integer(i128::from(v)))
    }

    fn visit_i128<E: de::Error>(self, v: i128) -> Result<Value, E> {
        Value::integer(v).ok_or_else(|| de::Error::custom("Value::Integer out of CBOR range"))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Integer(i128::from(v)))
    }

    fn visit_u128<E: de::Error>(self, v: u128) -> Result<Value, E> {
        i128::try_from(v)
            .ok()
            .and_then(Value::integer)
            .ok_or_else(|| de::Error::custom("Value::Integer out of CBOR range"))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        Ok(Value::Float(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::Text(v.to_string()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::Text(v))
    }

    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Value, E> {
        Ok(Value::Bytes(v.to_vec()))
    }

    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Value, E> {
        Ok(Value::Bytes(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(elem) = seq.next_element::<Value>()? {
            out.push(elem);
        }
        Ok(Value::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        // Pull the first key. If it's the sentinel, decode the special form. Otherwise it's a
        // plain string-keyed map and we continue collecting entries.
        let Some(first_key) = map.next_key::<String>()? else {
            return Ok(Value::Map(Vec::new()));
        };

        if first_key == SENTINEL_KEY {
            let kind: String = map.next_value()?;
            return decode_sentinel(&kind, &mut map);
        }

        if first_key == JSON_NUMBER_SENTINEL {
            let raw: String = map.next_value()?;
            return parse_json_arbitrary_number(&raw).map_err(de::Error::custom);
        }

        let first_value: Value = map.next_value()?;
        let mut entries: Vec<(Value, Value)> = Vec::with_capacity(map.size_hint().unwrap_or(0) + 1);
        entries.push((Value::Text(first_key), first_value));
        while let Some((k, v)) = map.next_entry::<String, Value>()? {
            entries.push((Value::Text(k), v));
        }
        Ok(Value::Map(entries))
    }
}

fn decode_sentinel<'de, A: MapAccess<'de>>(kind: &str, map: &mut A) -> Result<Value, A::Error> {
    match kind {
        SENTINEL_BYTES => {
            let mut hex_value: Option<String> = None;
            while let Some(k) = map.next_key::<String>()? {
                if k == "hex" {
                    hex_value = Some(map.next_value()?);
                } else {
                    let _: serde::de::IgnoredAny = map.next_value()?;
                }
            }
            let hex = hex_value.ok_or_else(|| de::Error::custom("missing 'hex' in @cbor:bytes"))?;
            decode_hex(&hex).map(Value::Bytes).map_err(de::Error::custom)
        },
        SENTINEL_INT => {
            let mut value_str: Option<String> = None;
            while let Some(k) = map.next_key::<String>()? {
                if k == "value" {
                    value_str = Some(map.next_value()?);
                } else {
                    let _: serde::de::IgnoredAny = map.next_value()?;
                }
            }
            let s = value_str.ok_or_else(|| de::Error::custom("missing 'value' in @cbor:int"))?;
            let i = s.parse::<i128>().map_err(de::Error::custom)?;
            Value::integer(i).ok_or_else(|| de::Error::custom("Value::Integer out of CBOR range"))
        },
        SENTINEL_MAP => {
            let mut entries: Option<Vec<(Value, Value)>> = None;
            while let Some(k) = map.next_key::<String>()? {
                if k == "entries" {
                    entries = Some(map.next_value()?);
                } else {
                    let _: serde::de::IgnoredAny = map.next_value()?;
                }
            }
            entries
                .map(Value::Map)
                .ok_or_else(|| de::Error::custom("missing 'entries' in @cbor:map"))
        },
        SENTINEL_TAG => {
            let mut tag: Option<u64> = None;
            let mut value: Option<Value> = None;
            while let Some(k) = map.next_key::<String>()? {
                match k.as_str() {
                    "tag" => tag = Some(map.next_value()?),
                    "value" => value = Some(map.next_value()?),
                    _ => {
                        let _: serde::de::IgnoredAny = map.next_value()?;
                    },
                }
            }
            let tag = tag.ok_or_else(|| de::Error::custom("missing 'tag' in @cbor:tag"))?;
            let value = value.ok_or_else(|| de::Error::custom("missing 'value' in @cbor:tag"))?;
            Ok(Value::Tag(tag, Box::new(value)))
        },
        other => Err(de::Error::custom(format!("unknown @cbor sentinel kind: {other}"))),
    }
}

/// Decode the string payload of a `$serde_json::private::Number` map. Integers prefer
/// `Value::Integer` (lossless for the i128 range); anything else falls back to `Value::Float`.
fn parse_json_arbitrary_number(raw: &str) -> Result<Value, String> {
    if let Ok(i) = raw.parse::<i128>() {
        return Value::integer(i).ok_or_else(|| format!("integer out of CBOR range: {raw}"));
    }
    raw.parse::<f64>()
        .map(Value::Float)
        .map_err(|_| format!("invalid arbitrary_precision number: {raw}"))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(hex_digit(b >> 4));
        out.push(hex_digit(b & 0x0f));
    }
    out
}

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + n - 10) as char,
        _ => unreachable!(),
    }
}

fn decode_hex(s: &str) -> Result<Vec<u8>, &'static str> {
    if !s.len().is_multiple_of(2) {
        return Err("odd-length hex string");
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks_exact(2) {
        let hi = parse_nibble(chunk[0])?;
        let lo = parse_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn parse_nibble(b: u8) -> Result<u8, &'static str> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err("invalid hex character"),
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;

    fn json_roundtrip(v: &Value) -> Value {
        let s = serde_json::to_string(v).unwrap();
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn primitive_round_trips() {
        for v in [
            Value::Null,
            Value::Bool(true),
            Value::Bool(false),
            Value::Integer(0),
            Value::Integer(-1),
            Value::Integer(i64::MAX.into()),
            Value::Integer(i64::MIN.into()),
            Value::Float(PI),
            Value::Text("hello world".into()),
        ] {
            assert_eq!(json_roundtrip(&v), v);
        }
    }

    #[test]
    fn integers_outside_i64_but_inside_the_cbor_range_round_trip() {
        for v in [
            Value::Integer(i128::from(u64::MAX)),
            Value::Integer(i128::from(i64::MAX) + 1),
        ] {
            assert_eq!(json_roundtrip(&v), v);
        }
    }

    // `Value::Integer` is constructible with any `i128`, so serialization stays total for one
    // outside [`INTEGER_RANGE`] and writes the sentinel. Reading it back would rebuild a value no
    // encoder can write, so every route into the type refuses it.
    #[test]
    fn an_integer_outside_the_cbor_range_is_refused_on_the_way_back() {
        let v = Value::Integer(i128::from(u64::MAX) + 1);
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("@cbor"));
        assert!(serde_json::from_str::<Value>(&json).is_err());
    }

    // The CBOR serde route reaches `Value` through `deserialize_any`, which hands a 64-bit negative
    // to `visit_i128` rather than through the minicbor decoder.
    #[test]
    fn the_serde_decoder_refuses_an_integer_below_i64_min() {
        const NEGATIVE_BELOW_I64_MIN: [u8; 9] = [0x3b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
        assert!(crate::serde_codec::from_slice::<Value>(&NEGATIVE_BELOW_I64_MIN).is_err());
    }

    #[test]
    fn bytes_uses_sentinel_and_round_trips() {
        let v = Value::Bytes(vec![0, 1, 2, 254, 255]);
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("@cbor"));
        assert!(json.contains("bytes"));
        assert_eq!(json_roundtrip(&v), v);
    }

    #[test]
    fn array_round_trips() {
        let v = Value::Array(vec![Value::Integer(1), Value::Text("a".into()), Value::Bool(true)]);
        assert_eq!(json_roundtrip(&v), v);
    }

    #[test]
    fn string_keyed_map_uses_native_json_object() {
        let v = Value::Map(vec![
            (Value::Text("a".into()), Value::Integer(1)),
            (Value::Text("b".into()), Value::Integer(2)),
        ]);
        let json = serde_json::to_string(&v).unwrap();
        assert!(!json.contains("@cbor"));
        assert_eq!(json_roundtrip(&v), v);
    }

    fn text_map(entries: &[(&str, Value)]) -> Value {
        Value::Map(
            entries
                .iter()
                .map(|(k, v)| (Value::Text((*k).into()), v.clone()))
                .collect(),
        )
    }

    #[test]
    fn text_keyed_maps_holding_a_reserved_key_round_trip() {
        let maps = [
            text_map(&[
                (SENTINEL_KEY, Value::Text("hello".into())),
                ("symbol", Value::Text("X".into())),
            ]),
            text_map(&[
                (SENTINEL_KEY, Value::Text(SENTINEL_BYTES.into())),
                ("hex", Value::Text("c0ffee".into())),
                ("symbol", Value::Text("X".into())),
            ]),
            text_map(&[
                (SENTINEL_KEY, Value::Text(SENTINEL_MAP.into())),
                ("entries", Value::Array(vec![])),
            ]),
            text_map(&[(JSON_NUMBER_SENTINEL, Value::Text("42".into()))]),
            text_map(&[("a", Value::Integer(1)), (SENTINEL_KEY, Value::Text("hello".into()))]),
            text_map(&[(
                "outer",
                text_map(&[
                    (SENTINEL_KEY, Value::Text(SENTINEL_TAG.into())),
                    ("tag", Value::Integer(1)),
                ]),
            )]),
        ];
        for v in maps {
            let json = serde_json::to_string(&v).unwrap();
            assert!(json.contains(r#""@cbor":"map""#), "{json}");
            assert_eq!(json_roundtrip(&v), v, "{json}");
        }
    }

    // `serde_json::Value` sorts object keys, so a reserved key that is not first in the CBOR map
    // becomes the first key once the JSON passes through it.
    #[test]
    fn a_reserved_key_survives_an_intermediate_json_value() {
        let v = text_map(&[("a", Value::Integer(1)), (SENTINEL_KEY, Value::Text("hello".into()))]);
        let json: serde_json::Value = serde_json::to_value(&v).unwrap();
        assert_eq!(serde_json::from_value::<Value>(json).unwrap(), v);
    }

    #[test]
    fn decodes_serde_json_arbitrary_precision_float() {
        // Shape produced by serde_json when its `arbitrary_precision` feature is on.
        let raw = r#"{"$serde_json::private::Number":"2.14"}"#;
        let decoded: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(decoded, Value::Float(2.14));
    }

    #[test]
    fn decodes_serde_json_arbitrary_precision_integer() {
        let raw = r#"{"$serde_json::private::Number":"42"}"#;
        let decoded: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(decoded, Value::Integer(42));
    }

    #[test]
    fn decodes_serde_json_arbitrary_precision_negative_integer() {
        let raw = r#"{"$serde_json::private::Number":"-1"}"#;
        let decoded: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(decoded, Value::Integer(-1));
    }

    #[test]
    fn non_text_keyed_map_uses_sentinel() {
        let v = Value::Map(vec![(Value::Integer(0), Value::Text("zero".into()))]);
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("@cbor"));
        assert_eq!(json_roundtrip(&v), v);
    }

    #[test]
    fn tagged_value_round_trips() {
        let v = Value::Tag(42, Box::new(Value::Text("inner".into())));
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("@cbor"));
        assert!(json.contains("tag"));
        assert_eq!(json_roundtrip(&v), v);
    }

    #[test]
    fn nested_complex_round_trips() {
        let v = crate::cbor!({
            "code" => 415u32,
            "active" => false,
            "items" => [1u32, 2u32, 3u32],
            "tagged" => Value::Tag(100, Box::new(Value::Bytes(vec![1, 2, 3]))),
        });
        assert_eq!(json_roundtrip(&v), v);
    }

    #[test]
    fn null_round_trips_through_the_serde_cbor_codec() {
        let v = crate::cbor!({
            "missing" => null,
            "items" => [null, 1u32],
        });
        let bytes = crate::serde_codec::to_vec(&v).unwrap();
        assert_eq!(crate::serde_codec::from_slice::<Value>(&bytes).unwrap(), v);
    }
}
