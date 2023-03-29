//  Copyright 2023, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use anyhow::Context;
use tari_engine_types::{
    non_fungible::NonFungibleContainer,
    substate::{Substate, SubstateValue},
};
use tari_template_lib::models::ComponentHeader;

pub type JsonValue = serde_json::Value;
pub type JsonObject = serde_json::Map<String, JsonValue>;
pub type CborValue = ciborium::value::Value;

pub fn decode_substate_into_json(substate: &Substate) -> Result<JsonValue, anyhow::Error> {
    let substate_cbor = decode_into_cbor(&substate.to_bytes())?;
    let mut result = serde_json::to_value(substate_cbor)?;

    let substate_field = get_mut_json_field(&mut result, "substate")?;
    match substate.substate_value() {
        SubstateValue::NonFungible(nf_container) => {
            decode_non_fungible_into_json(nf_container, substate_field)?;
        },
        SubstateValue::Component(header) => {
            decode_component_into_json(header, substate_field)?;
        },
        _ => {},
    }

    Ok(result)
}

fn decode_non_fungible_into_json(
    nf_container: &NonFungibleContainer,
    substate_json_field: &mut JsonValue,
) -> Result<(), anyhow::Error> {
    if let Some(nf) = nf_container.contents() {
        let non_fungible_field = get_mut_json_field(substate_json_field, "NonFungible")?;
        let non_fungible_object = json_value_as_object(non_fungible_field)?;

        decode_cbor_field_into_json(nf.data(), non_fungible_object, "data")?;
        decode_cbor_field_into_json(nf.mutable_data(), non_fungible_object, "mutable_data")?;
    }

    Ok(())
}

fn decode_component_into_json(
    header: &ComponentHeader,
    substate_json_field: &mut JsonValue,
) -> Result<(), anyhow::Error> {
    let component_field = get_mut_json_field(substate_json_field, "Component")?;
    let component_object = json_value_as_object(component_field)?;
    decode_cbor_field_into_json(header.state(), component_object, "state")?;

    Ok(())
}

fn decode_cbor_field_into_json(
    bytes: &[u8],
    parent_object: &mut JsonObject,
    field_name: &str,
) -> Result<(), anyhow::Error> {
    let cbor_value = decode_into_cbor(bytes)?;
    let cbor_value = fix_invalid_object_keys(&cbor_value);
    let json_value = serde_json::to_value(cbor_value)?;
    parent_object.insert(field_name.to_owned(), json_value);

    Ok(())
}

// In JSON, all object keys must be string values.
// But ciborium sometimes will use other types (e.g. Tags) as keys,
// so in that case we transform the object into an array so it can be safely converted to JSON
// AND we need to to it recursively
fn fix_invalid_object_keys(value: &CborValue) -> CborValue {
    match value {
        ciborium::value::Value::Tag(tag, content) => {
            let fixed_content = fix_invalid_object_keys(content);
            ciborium::value::Value::Tag(*tag, Box::new(fixed_content))
        },
        ciborium::value::Value::Array(arr) => {
            let fixed_items = arr.iter().map(fix_invalid_object_keys).collect();
            ciborium::value::Value::Array(fixed_items)
        },
        ciborium::value::Value::Map(map) => {
            let has_invalid_keys = map.iter().any(|(k, _)| !k.is_text());

            if has_invalid_keys {
                let map_entries_as_arrays = map
                    .iter()
                    .map(|(k, v)| {
                        let fixed_key = fix_invalid_object_keys(k);
                        let fixed_value = fix_invalid_object_keys(v);
                        ciborium::value::Value::Array(vec![fixed_key, fixed_value])
                    })
                    .collect();
                return ciborium::value::Value::Array(map_entries_as_arrays);
            }

            let fixed_entries = map
                .iter()
                .map(|(k, v)| {
                    let fixed_value = fix_invalid_object_keys(v);
                    (k.to_owned(), fixed_value)
                })
                .collect();
            ciborium::value::Value::Map(fixed_entries)
        },
        // other types are atomic and do not cause problems, so we just return them directly
        _ => value.to_owned(),
    }
}

fn get_mut_json_field<'a>(value: &'a mut JsonValue, field_name: &str) -> Result<&'a mut JsonValue, anyhow::Error> {
    let json_field = json_value_as_object(value)?
        .get_mut(field_name)
        .context("field does not exist")?;

    Ok(json_field)
}

fn json_value_as_object(value: &mut JsonValue) -> Result<&mut JsonObject, anyhow::Error> {
    let json_object = value.as_object_mut().context("invalid object")?;

    Ok(json_object)
}

fn decode_into_cbor(bytes: &[u8]) -> Result<CborValue, anyhow::Error> {
    Ok(ciborium::de::from_reader::<CborValue, _>(bytes)?)
}
