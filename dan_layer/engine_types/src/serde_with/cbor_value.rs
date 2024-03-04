//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tari_bor::json_encoding::{CborValueJsonDeserializeWrapper, CborValueJsonSerializeWrapper};

pub fn serialize<S: Serializer>(v: &tari_bor::Value, s: S) -> Result<S::Ok, S::Error> {
    if s.is_human_readable() {
        CborValueJsonSerializeWrapper(v).serialize(s)
    } else {
        v.serialize(s)
    }
}

pub fn deserialize<'de, D>(d: D) -> Result<tari_bor::Value, D::Error>
where D: Deserializer<'de> {
    if d.is_human_readable() {
        let wrapper = CborValueJsonDeserializeWrapper::deserialize(d)?;
        Ok(wrapper.0)
    } else {
        tari_bor::Value::deserialize(d)
    }
}

#[cfg(test)]
mod tests {
    use tari_bor::cbor;
    use tari_template_lib::models::{ObjectKey, ResourceAddress};

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Test {
        #[serde(with = "super")]
        value: tari_bor::Value,
    }

    #[test]
    fn decode_encode() {
        let addr = ResourceAddress::new([1u8; ObjectKey::LENGTH].into());
        let test = Test {
            value: cbor!({
                "code" => 415,
                "message" => addr,
                "continue" => false,
                "array" => [1, 2, 3, 4, 5],
                "extra" => { "numbers" => [8.2341e+4, 0.251425] },
            })
            .unwrap(),
        };

        let json = serde_json::to_string(&test).unwrap();
        let t = serde_json::from_str::<Test>(&json).unwrap();
        assert_eq!(test, t);

        let json2 = serde_json::to_string(&t).unwrap();
        assert_eq!(json, json2);
    }
}
