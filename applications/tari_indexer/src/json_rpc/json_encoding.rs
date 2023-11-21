//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde_json as json;
use tari_engine_types::{
    commit_result::ExecuteResult,
    component::ComponentHeader,
    non_fungible::NonFungibleContainer,
    substate::{Substate, SubstateValue},
};
use tari_validator_node_rpc::client::FinalizedResult;

type JsonObject = json::Map<String, json::Value>;
type CborValue = tari_bor::Value;

#[derive(Debug, thiserror::Error)]
pub enum JsonEncodingError {
    #[error("Could not decode the substate: {0}")]
    BinaryEncoding(#[from] tari_bor::BorError),
    #[error("Serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("Unexpected error: {0}")]
    Unexpected(String),
}

pub fn encode_finalized_result_into_json(result: &FinalizedResult) -> Result<Vec<json::Value>, JsonEncodingError> {
    match &result.execute_result {
        Some(res) => encode_execute_result_into_json(res),
        None => Ok(vec![]),
    }
}

pub fn encode_execute_result_into_json(result: &ExecuteResult) -> Result<Vec<json::Value>, JsonEncodingError> {
    result
        .finalize
        .execution_results
        .iter()
        .map(|r| serde_json::to_value(r.indexed.value()).map_err(JsonEncodingError::Serde))
        .collect()
}

pub fn encode_substate_into_json(substate: &Substate) -> Result<json::Value, JsonEncodingError> {
    let substate_cbor = tari_bor::to_value(&substate)?;
    let substate_cbor = fix_invalid_object_keys(&substate_cbor);
    let mut result = json::to_value(substate_cbor)?;

    let substate_field = get_mut_json_field(&mut result, "substate")?;
    match substate.substate_value() {
        SubstateValue::NonFungible(nf_container) => {
            encode_non_fungible_into_json(nf_container, substate_field)?;
        },
        SubstateValue::Component(header) => {
            encode_component_into_json(header, substate_field)?;
        },
        _ => {},
    }

    Ok(result)
}

fn get_mut_json_field<'a>(
    value: &'a mut json::Value,
    field_name: &str,
) -> Result<&'a mut json::Value, JsonEncodingError> {
    let json_field = json_value_as_object(value)?
        .get_mut(field_name)
        .ok_or(JsonEncodingError::Unexpected("field does not exist".to_owned()))?;

    Ok(json_field)
}

fn json_value_as_object(value: &mut json::Value) -> Result<&mut JsonObject, JsonEncodingError> {
    let json_object = value
        .as_object_mut()
        .ok_or(JsonEncodingError::Unexpected("invalid object".to_owned()))?;

    Ok(json_object)
}

fn encode_non_fungible_into_json(
    nf_container: &NonFungibleContainer,
    substate_json_field: &mut json::Value,
) -> Result<(), JsonEncodingError> {
    if let Some(nf) = nf_container.contents() {
        let non_fungible_field = get_mut_json_field(substate_json_field, "NonFungible")?;
        let non_fungible_object = json_value_as_object(non_fungible_field)?;

        decode_cbor_field_into_json(nf.data(), non_fungible_object, "data")?;
        decode_cbor_field_into_json(nf.mutable_data(), non_fungible_object, "mutable_data")?;
    }

    Ok(())
}

fn decode_cbor_field_into_json(
    bytes: &[u8],
    parent_object: &mut JsonObject,
    field_name: &str,
) -> Result<(), JsonEncodingError> {
    let cbor_value = tari_bor::decode(bytes)?;
    fix_cbor_value_for_json(&cbor_value, parent_object, field_name)
}

fn fix_cbor_value_for_json(
    cbor_value: &tari_bor::Value,
    parent_object: &mut JsonObject,
    field_name: &str,
) -> Result<(), JsonEncodingError> {
    let cbor_value = fix_invalid_object_keys(cbor_value);
    let json_value = serde_json::to_value(cbor_value)?;
    parent_object.insert(field_name.to_owned(), json_value);

    Ok(())
}

fn encode_component_into_json(
    header: &ComponentHeader,
    substate_json_field: &mut json::Value,
) -> Result<(), JsonEncodingError> {
    let component_field = get_mut_json_field(substate_json_field, "Component")?;
    let component_object = json_value_as_object(component_field)?;
    fix_cbor_value_for_json(header.state(), component_object, "state")?;

    Ok(())
}

/// In JSON, all object keys must be string values.
/// But ciborium sometimes will use other types (e.g. Tags) as keys,
/// so in that case we transform the object into an array so it can be safely converted to JSON
/// AND we need to to it recursively
fn fix_invalid_object_keys(value: &CborValue) -> CborValue {
    match value {
        CborValue::Tag(tag, content) => {
            let fixed_content = fix_invalid_object_keys(content);
            CborValue::Tag(*tag, Box::new(fixed_content))
        },
        CborValue::Array(arr) => {
            let fixed_items = arr.iter().map(fix_invalid_object_keys).collect();
            CborValue::Array(fixed_items)
        },
        CborValue::Map(map) => {
            let has_invalid_keys = map.iter().any(|(k, _)| !k.is_text());

            if has_invalid_keys {
                let map_entries_as_arrays = map
                    .iter()
                    .map(|(k, v)| {
                        let fixed_key = fix_invalid_object_keys(k);
                        let fixed_value = fix_invalid_object_keys(v);
                        CborValue::Array(vec![fixed_key, fixed_value])
                    })
                    .collect();
                return CborValue::Array(map_entries_as_arrays);
            }

            let fixed_entries = map
                .iter()
                .map(|(k, v)| {
                    let fixed_value = fix_invalid_object_keys(v);
                    (k.clone(), fixed_value)
                })
                .collect();
            CborValue::Map(fixed_entries)
        },
        // other types are atomic and do not cause problems, so we just return them directly
        _ => value.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use tari_common_types::types::Commitment;
    use tari_engine_types::{confidential::ConfidentialOutput, resource_container::ResourceContainer, vault::Vault};
    use tari_template_lib::{
        models::{Amount, ResourceAddress, VaultId},
        Hash,
    };

    use super::*;

    #[test]
    fn it_encodes_confidential_vaults() {
        let address = ResourceAddress::new(Hash::default());

        let commitment = Commitment::default();
        let confidential_output = ConfidentialOutput {
            commitment: commitment.clone(),
            stealth_public_nonce: commitment.as_public_key().clone(),
            encrypted_data: Default::default(),
            minimum_value_promise: 0,
        };
        let commitment = Some((commitment, confidential_output));

        let revealed_amount = Amount::zero();
        let container = ResourceContainer::confidential(address, commitment, revealed_amount);

        let vault_id = VaultId::new(Hash::default());
        let vault = Vault::new(vault_id, container);

        let substate_value = SubstateValue::Vault(vault);
        let substate = Substate::new(0, substate_value);

        assert!(encode_substate_into_json(&substate).is_ok());
    }
}
