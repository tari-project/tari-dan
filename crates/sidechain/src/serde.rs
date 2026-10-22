// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::borrow::Cow;

use serde::{Deserialize, Deserializer, Serializer};
/// Encodes a byte type as a hex string if the serializer is human-readable, Otherwise efficiently encodes it as
/// bytes.
/// This is different from the standard serde byte encoding, which encodes a sequence of u8s (e.g. [1, 2, 3]) as a JSON
/// array.
pub mod hex_or_bytes {
    use super::*;

    pub fn serialize<S: Serializer, T: AsRef<[u8]>>(v: &T, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            let st = ::hex::encode(v.as_ref());
            s.serialize_str(&st)
        } else {
            s.serialize_bytes(v.as_ref())
        }
    }

    /// Use a serde deserializer to serialize the hex string of the given object.
    pub fn deserialize<'de, D, T>(d: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: for<'a> TryFrom<&'a [u8]>,
    {
        let value = if d.is_human_readable() {
            let hex = <Cow<'_, str> as Deserialize>::deserialize(d)?;
            let bytes = ::hex::decode(&*hex).map_err(serde::de::Error::custom)?;
            T::try_from(&bytes).map_err(|_| serde::de::Error::custom("Failed to convert bytes to T"))?
        } else {
            let bytes = <Cow<'_, [u8]> as Deserialize>::deserialize(d)?;
            T::try_from(&bytes).map_err(|_| serde::de::Error::custom("Failed to convert bytes to T"))?
        };

        Ok(value)
    }
}

pub mod hex {
    use tari_utilities::hex::{from_hex, to_hex};

    use super::*;

    pub fn serialize<S: Serializer, T: AsRef<[u8]>>(v: &T, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            s.serialize_str(&to_hex(v.as_ref()))
        } else {
            s.serialize_bytes(v.as_ref())
        }
    }

    pub fn deserialize<'de, D, T>(d: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: TryFrom<Vec<u8>>,
    {
        let bytes = if d.is_human_readable() {
            let hex = <String as Deserialize>::deserialize(d)?;
            from_hex(&hex).map_err(serde::de::Error::custom)?
        } else {
            <Vec<u8> as Deserialize>::deserialize(d)?
        };

        T::try_from(bytes).map_err(|_| serde::de::Error::custom("Failed to convert bytes to T"))
    }
}

pub mod hex_seq {
    use serde::Serialize;

    use super::*;

    pub fn serialize<S: Serializer, T: AsRef<[u8]>>(v: &[T], s: S) -> Result<S::Ok, S::Error> {
        struct Item<'a>(&'a [u8]);

        impl Serialize for Item<'_> {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                super::hex::serialize(&self.0, s)
            }
        }

        s.collect_seq(v.iter().map(|item| Item(item.as_ref())))
    }

    pub fn deserialize<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: TryFrom<Vec<u8>>,
    {
        struct Item<T>(T);

        impl<'de, T: TryFrom<Vec<u8>>> Deserialize<'de> for Item<T> {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                super::hex::deserialize(d).map(Item)
            }
        }

        let items = Vec::<Item<T>>::deserialize(d)?;
        Ok(items.into_iter().map(|item| item.0).collect())
    }
}

pub mod base64 {
    use ::base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::*;

    pub fn serialize<S: Serializer, T: AsRef<[u8]>>(v: &T, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            s.serialize_str(&STANDARD.encode(v))
        } else {
            s.serialize_bytes(v.as_ref())
        }
    }

    pub fn deserialize<'de, D, T>(d: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: TryFrom<Vec<u8>>,
    {
        let bytes = if d.is_human_readable() {
            let s = String::deserialize(d)?;
            STANDARD.decode(s.as_bytes()).map_err(serde::de::Error::custom)?
        } else {
            Vec::<u8>::deserialize(d)?
        };

        T::try_from(bytes).map_err(|_| serde::de::Error::custom("Failed to convert bytes to T"))
    }
}
