//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use core::{fmt, fmt::Formatter, marker::PhantomData};

use ciborium::{value::Integer, Value};
use serde::{de::VariantAccess, ser::SerializeTupleVariant};

pub struct CborValueJsonSerializeWrapper<'a>(pub &'a Value);

impl serde::Serialize for CborValueJsonSerializeWrapper<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        match self.0 {
            Value::Integer(ref __field0) => {
                let value = i128::from(*__field0);
                serializer.serialize_newtype_variant("Value", 1u32, "Integer", &value)
            },
            Value::Bytes(ref __field0) => {
                serde::Serializer::serialize_newtype_variant(serializer, "Value", 1u32, "Bytes", __field0)
            },
            Value::Float(ref __field0) => {
                serde::Serializer::serialize_newtype_variant(serializer, "Value", 2u32, "Float", __field0)
            },
            Value::Text(ref __field0) => {
                serde::Serializer::serialize_newtype_variant(serializer, "Value", 3u32, "Text", __field0)
            },
            Value::Bool(ref __field0) => {
                serde::Serializer::serialize_newtype_variant(serializer, "Value", 4u32, "Bool", __field0)
            },
            Value::Null => serde::Serializer::serialize_unit_variant(serializer, "Value", 5u32, "Null"),
            Value::Tag(ref tag, ref value) => {
                let mut state = serde::Serializer::serialize_tuple_variant(serializer, "Value", 6u32, "Tag", 2)?;
                SerializeTupleVariant::serialize_field(&mut state, tag)?;
                SerializeTupleVariant::serialize_field(&mut state, &Self(value))?;
                SerializeTupleVariant::end(state)
            },
            Value::Array(ref arr) => {
                let wrapped = arr.iter().map(Self).collect::<Vec<_>>();
                serde::Serializer::serialize_newtype_variant(serializer, "Value", 7u32, "Array", &wrapped)
            },
            Value::Map(ref map) => {
                let wrapped = map.iter().map(|(k, v)| (Self(k), Self(v))).collect::<Vec<_>>();
                serde::Serializer::serialize_newtype_variant(serializer, "Value", 8u32, "Map", &wrapped)
            },
            ref v => Err(serde::ser::Error::custom(format!("invalid value {:?}", v))),
        }
    }
}

pub struct CborValueJsonDeserializeWrapper(pub Value);

impl CborValueJsonDeserializeWrapper {
    pub fn into_inner(self) -> Value {
        self.0
    }
}

impl<'de> serde::Deserialize<'de> for CborValueJsonDeserializeWrapper {
    #[allow(clippy::too_many_lines)]
    fn deserialize<__D>(__deserializer: __D) -> Result<Self, __D::Error>
    where __D: serde::Deserializer<'de> {
        #[allow(non_camel_case_types)]
        #[doc(hidden)]
        enum __Field {
            Integer,
            Bytes,
            Float,
            Text,
            Bool,
            Null,
            Tag,
            Array,
            Map,
        }
        #[doc(hidden)]
        struct __FieldVisitor;
        impl<'de> serde::de::Visitor<'de> for __FieldVisitor {
            type Value = __Field;

            fn expecting(&self, __formatter: &mut Formatter) -> fmt::Result {
                Formatter::write_str(__formatter, "variant identifier")
            }

            fn visit_u64<__E>(self, __value: u64) -> Result<Self::Value, __E>
            where __E: serde::de::Error {
                match __value {
                    0u64 => Ok(__Field::Integer),
                    1u64 => Ok(__Field::Bytes),
                    2u64 => Ok(__Field::Float),
                    3u64 => Ok(__Field::Text),
                    4u64 => Ok(__Field::Bool),
                    5u64 => Ok(__Field::Null),
                    6u64 => Ok(__Field::Tag),
                    7u64 => Ok(__Field::Array),
                    8u64 => Ok(__Field::Map),
                    _ => Err(serde::de::Error::invalid_value(
                        serde::de::Unexpected::Unsigned(__value),
                        &"variant index 0 <= i < 9",
                    )),
                }
            }

            fn visit_str<__E>(self, __value: &str) -> Result<Self::Value, __E>
            where __E: serde::de::Error {
                match __value {
                    "Integer" => Ok(__Field::Integer),
                    "Bytes" => Ok(__Field::Bytes),
                    "Float" => Ok(__Field::Float),
                    "Text" => Ok(__Field::Text),
                    "Bool" => Ok(__Field::Bool),
                    "Null" => Ok(__Field::Null),
                    "Tag" => Ok(__Field::Tag),
                    "Array" => Ok(__Field::Array),
                    "Map" => Ok(__Field::Map),
                    _ => Err(serde::de::Error::unknown_variant(__value, VARIANTS)),
                }
            }

            fn visit_bytes<__E>(self, __value: &[u8]) -> Result<Self::Value, __E>
            where __E: serde::de::Error {
                match __value {
                    b"Integer" => Ok(__Field::Integer),
                    b"Bytes" => Ok(__Field::Bytes),
                    b"Float" => Ok(__Field::Float),
                    b"Text" => Ok(__Field::Text),
                    b"Bool" => Ok(__Field::Bool),
                    b"Null" => Ok(__Field::Null),
                    b"Tag" => Ok(__Field::Tag),
                    b"Array" => Ok(__Field::Array),
                    b"Map" => Ok(__Field::Map),
                    _ => {
                        let __value = String::from_utf8_lossy(__value);
                        Err(serde::de::Error::unknown_variant(&__value, VARIANTS))
                    },
                }
            }
        }
        impl<'de> serde::Deserialize<'de> for __Field {
            #[inline]
            fn deserialize<__D>(__deserializer: __D) -> Result<Self, __D::Error>
            where __D: serde::Deserializer<'de> {
                serde::Deserializer::deserialize_identifier(__deserializer, __FieldVisitor)
            }
        }
        #[doc(hidden)]
        struct __Visitor<'de> {
            marker: PhantomData<CborValueJsonDeserializeWrapper>,
            lifetime: PhantomData<&'de ()>,
        }
        impl<'de> serde::de::Visitor<'de> for __Visitor<'de> {
            type Value = CborValueJsonDeserializeWrapper;

            fn expecting(&self, __formatter: &mut Formatter) -> fmt::Result {
                Formatter::write_str(__formatter, "enum Value")
            }

            fn visit_enum<__A>(self, __data: __A) -> Result<Self::Value, __A::Error>
            where __A: serde::de::EnumAccess<'de> {
                match serde::de::EnumAccess::variant(__data)? {
                    (__Field::Integer, __variant) => {
                        let a = __variant.newtype_variant::<i128>()?;
                        Ok(CborValueJsonDeserializeWrapper(Value::Integer(
                            Integer::try_from(a).map_err(serde::de::Error::custom)?,
                        )))
                    },
                    (__Field::Bytes, __variant) => __variant
                        .newtype_variant()
                        .map(Value::Bytes)
                        .map(CborValueJsonDeserializeWrapper),
                    (__Field::Float, __variant) => __variant
                        .newtype_variant()
                        .map(Value::Float)
                        .map(CborValueJsonDeserializeWrapper),
                    (__Field::Text, __variant) => __variant
                        .newtype_variant()
                        .map(Value::Text)
                        .map(CborValueJsonDeserializeWrapper),
                    (__Field::Bool, __variant) => __variant
                        .newtype_variant()
                        .map(Value::Bool)
                        .map(CborValueJsonDeserializeWrapper),
                    (__Field::Null, __variant) => {
                        __variant.unit_variant()?;
                        Ok(CborValueJsonDeserializeWrapper(Value::Null))
                    },
                    (__Field::Tag, __variant) => {
                        #[doc(hidden)]
                        struct __Visitor<'de> {
                            marker: PhantomData<CborValueJsonDeserializeWrapper>,
                            lifetime: PhantomData<&'de ()>,
                        }
                        impl<'de> serde::de::Visitor<'de> for __Visitor<'de> {
                            type Value = CborValueJsonDeserializeWrapper;

                            fn expecting(&self, __formatter: &mut Formatter) -> fmt::Result {
                                Formatter::write_str(__formatter, "tuple variant Value::Tag")
                            }

                            #[inline]
                            fn visit_seq<__A>(self, mut __seq: __A) -> Result<Self::Value, __A::Error>
                            where __A: serde::de::SeqAccess<'de> {
                                let __field0 = match serde::de::SeqAccess::next_element::<u64>(&mut __seq)? {
                                    Some(__value) => __value,
                                    None => {
                                        return Err(serde::de::Error::invalid_length(
                                            0usize,
                                            &"tuple variant Value::Tag with 2 elements",
                                        ))
                                    },
                                };
                                let wrapped =
                                    serde::de::SeqAccess::next_element::<CborValueJsonDeserializeWrapper>(&mut __seq)?
                                        .ok_or_else(|| {
                                            serde::de::Error::invalid_length(
                                                1usize,
                                                &"tuple variant Value::Tag with 2 elements",
                                            )
                                        })?;
                                Ok(CborValueJsonDeserializeWrapper(Value::Tag(
                                    __field0,
                                    Box::new(wrapped.0),
                                )))
                            }
                        }
                        VariantAccess::tuple_variant(__variant, 2usize, __Visitor {
                            marker: PhantomData,
                            lifetime: PhantomData,
                        })
                    },
                    (__Field::Array, __variant) => {
                        let values = __variant.newtype_variant::<Vec<CborValueJsonDeserializeWrapper>>()?;
                        Ok(CborValueJsonDeserializeWrapper(Value::Array(
                            values.into_iter().map(|v| v.0).collect(),
                        )))
                    },
                    (__Field::Map, __variant) => {
                        let values = __variant
                            .newtype_variant::<Vec<(CborValueJsonDeserializeWrapper, CborValueJsonDeserializeWrapper)>>(
                            )?;
                        Ok(CborValueJsonDeserializeWrapper(Value::Map(
                            values.into_iter().map(|(k, v)| (k.0, v.0)).collect(),
                        )))
                    },
                }
            }
        }
        #[doc(hidden)]
        const VARIANTS: &[&str] = &[
            "Integer", "Bytes", "Float", "Text", "Bool", "Null", "Tag", "Array", "Map",
        ];
        serde::Deserializer::deserialize_enum(__deserializer, "Value", VARIANTS, __Visitor {
            marker: PhantomData,
            lifetime: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use ciborium::cbor;

    use super::*;

    #[test]
    fn decode_encode() {
        let bytes = [1u8; 32];
        let sample = cbor!({
            "code" => 415,
            "message" => null,
            "continue" => false,
            // Non-string keys
            "bytes" => bytes,
            "array" => [{()=>1}, {123=>2}, {cbor!({"A"=>1}).unwrap()=>3}],
            "extra" => { "numbers" => [8.2341e+4, 0.251425] },
        })
        .unwrap();
        let s1 = serde_json::to_string(&CborValueJsonSerializeWrapper(&sample)).unwrap();
        let decoded = serde_json::from_str::<CborValueJsonDeserializeWrapper>(&s1).unwrap();
        // Decoded value matches original
        assert_eq!(sample, decoded.0);
        let s2 = serde_json::to_string(&CborValueJsonSerializeWrapper(&decoded.0)).unwrap();
        // Re-encode to string, which should be identical to the previously encoded JSON string
        assert_eq!(s1, s2);
    }
}
