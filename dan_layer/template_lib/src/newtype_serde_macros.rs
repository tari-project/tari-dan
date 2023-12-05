//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

/// Low-level macro used for generating serde serializers/deserializers for newtype structs. Not intended for general usage
#[macro_export]
macro_rules! newtype_struct_serde_impl {
    ($ty:ident, $inner_ty:ty) => {
        mod __serde_impl {
            use serde::{
                __private::{fmt, PhantomData},
                de::Error,
            };

            use super::*;

            impl serde::Serialize for $ty {
                fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                where S: serde::Serializer {
                    if serializer.is_human_readable() {
                        serializer.serialize_str(&self.to_string())
                    } else {
                        serializer.serialize_newtype_struct("$ty", &self.0)
                    }
                }
            }

            impl<'de> serde::Deserialize<'de> for $ty {
                fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
                where D: serde::Deserializer<'de> {
                    #[doc(hidden)]
                    struct Visitor<'de> {
                        marker: PhantomData<$ty>,
                        lifetime: PhantomData<&'de ()>,
                    }
                    impl<'de> serde::de::Visitor<'de> for Visitor<'de> {
                        type Value = $ty;

                        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                            fmt::Formatter::write_str(formatter, concat!("tuple struct ", stringify!($ty)))
                        }

                        fn visit_newtype_struct<E>(self, __e: E) -> Result<Self::Value, E::Error>
                        where E: serde::Deserializer<'de> {
                            let __field0: $inner_ty = match (<$inner_ty as serde::Deserialize>::deserialize(__e)) {
                                Ok(__val) => __val,
                                Err(__err) => {
                                    return Err(__err);
                                },
                            };
                            Ok($ty(__field0))
                        }

                        fn visit_seq<A>(self, mut __seq: A) -> Result<Self::Value, A::Error>
                        where A: serde::de::SeqAccess<'de> {
                            let __field0 = match match (serde::de::SeqAccess::next_element::<$inner_ty>(&mut __seq)) {
                                Ok(__val) => __val,
                                Err(__err) => {
                                    return Err(__err);
                                },
                            } {
                                Some(__value) => __value,
                                None => {
                                    return Err(serde::de::Error::invalid_length(
                                        0usize,
                                        &concat!("tuple struct ", stringify!($ty), " with 1 element"),
                                    ));
                                },
                            };
                            Ok($ty(__field0))
                        }

                        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
                        where E: Error {
                            <$ty>::from_str(v).map_err(|_| Error::custom(concat!("Invalid ", stringify!($ty))))
                        }
                    }
                    if deserializer.is_human_readable() {
                        deserializer.deserialize_str(Visitor {
                            marker: PhantomData::<$ty>,
                            lifetime: PhantomData,
                        })
                    } else {
                        deserializer.deserialize_newtype_struct(stringify!($ty), Visitor {
                            marker: PhantomData::<$ty>,
                            lifetime: PhantomData,
                        })
                    }
                }
            }
        }
    };
}
