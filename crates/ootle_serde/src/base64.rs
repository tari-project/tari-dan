//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use alloc::{format, string::String, vec::Vec};
use core::any;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Deserializer, Serializer};

use crate::visitor::BytesVisitor;

pub fn serialize<S: Serializer, T: AsRef<[u8]>>(v: &T, s: S) -> Result<S::Ok, S::Error> {
    if s.is_human_readable() {
        let base64 = STANDARD.encode(v);
        s.serialize_str(&base64)
    } else {
        s.serialize_bytes(v.as_ref())
    }
}

pub fn deserialize<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    for<'a> T: TryFrom<Vec<u8>>,
{
    if d.is_human_readable() {
        let s = String::deserialize(d)?;
        let bytes = STANDARD.decode(s.as_bytes()).map_err(serde::de::Error::custom)?;
        T::try_from(bytes).map_err(|_| {
            serde::de::Error::custom(format!("base64 Failed to convert bytes to {}", any::type_name::<T>()))
        })
    } else {
        let bytes = d.deserialize_byte_buf(BytesVisitor::new())?;
        T::try_from(bytes.into()).map_err(|_| {
            serde::de::Error::custom(format!(
                "base64: Failed to convert base64 bytes to {}",
                any::type_name::<T>()
            ))
        })
    }
}

pub mod option {
    use alloc::vec::Vec;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    struct Encode<'a, T>(&'a T);

    impl<T: AsRef<[u8]>> Serialize for Encode<'_, T> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            super::serialize(self.0, s)
        }
    }

    struct Decode<T>(T);

    impl<'de, T: TryFrom<Vec<u8>>> Deserialize<'de> for Decode<T> {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            super::deserialize(d).map(Decode)
        }
    }

    pub fn serialize<S: Serializer, T: AsRef<[u8]>>(v: &Option<T>, s: S) -> Result<S::Ok, S::Error> {
        v.as_ref().map(Encode).serialize(s)
    }

    pub fn deserialize<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: TryFrom<Vec<u8>>,
    {
        Ok(Option::<Decode<T>>::deserialize(d)?.map(|Decode(v)| v))
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::*;
    use crate::test_fixtures::Bytes32;

    #[derive(Deserialize, Serialize, PartialEq, Debug)]
    struct SampleData {
        #[serde(with = "super")]
        data: Vec<u8>,
        #[serde(with = "super")]
        hash: Bytes32,
    }

    #[test]
    fn it_encodes_and_decodes_from_base64() {
        let original = SampleData {
            data: vec![1, 2, 3, 4, 5],
            hash: Bytes32::new([123u8; 32]),
        };

        // Serialize to JSON (human-readable)
        let json = serde_json::to_value(&original).unwrap();
        json["data"].as_str().expect("data should be a string");
        json["hash"].as_str().expect("hash should be a string");

        // Deserialize from JSON
        let deserialized: SampleData = serde_json::from_value(json).unwrap();
        assert_eq!(original, deserialized);

        // Serialize to binary (non-human-readable)
        let binary = tari_bor::serde_codec::to_vec(&original).unwrap();

        // Deserialize from binary
        let deserialized_bin: SampleData = tari_bor::serde_codec::from_slice(&binary).unwrap();
        assert_eq!(original, deserialized_bin);
    }

    #[derive(Deserialize, Serialize, PartialEq, Debug)]
    struct OptionalData {
        #[serde(with = "super::option")]
        data: Option<Vec<u8>>,
    }

    #[test]
    fn an_optional_value_encodes_as_base64_or_null() {
        for original in [
            OptionalData {
                data: Some(vec![4, 5, 6]),
            },
            OptionalData { data: None },
        ] {
            let json = serde_json::to_value(&original).unwrap();
            assert_eq!(
                json["data"],
                original
                    .data
                    .as_ref()
                    .map_or(serde_json::Value::Null, |_| serde_json::Value::from("BAUG"))
            );
            assert_eq!(serde_json::from_value::<OptionalData>(json).unwrap(), original);

            let binary = tari_bor::serde_codec::to_vec(&original).unwrap();
            assert_eq!(
                tari_bor::serde_codec::from_slice::<OptionalData>(&binary).unwrap(),
                original
            );
        }
    }
}
