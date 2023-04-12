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
    substate::{Substate, SubstateAddress, SubstateValue},
};
use tari_template_lib::{
    models::{BinaryTag, ComponentHeader, VaultId},
    prelude::{ComponentAddress, NonFungibleAddress, ResourceAddress},
};

pub type JsonValue = serde_json::Value;
pub type JsonObject = serde_json::Map<String, JsonValue>;
pub type CborValue = ciborium::value::Value;

pub fn decode_substate_into_json(substate: &Substate) -> Result<JsonValue, anyhow::Error> {
    let substate_cbor = decode_into_cbor(&substate.to_bytes())?;
    let substate_cbor = fix_invalid_object_keys(&substate_cbor);
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

// Recursively scan a substate for references to other substates
pub fn find_related_substates(substate: &Substate) -> Result<Vec<SubstateAddress>, anyhow::Error> {
    match substate.substate_value() {
        SubstateValue::Component(header) => {
            // a component "state" is encoded using CBOR, so we need to decode and then scan inside for references
            let cbor_state = decode_into_cbor(header.state())?;
            let related_substates = find_related_substates_in_cbor_value(&cbor_state)?;
            Ok(related_substates)
        },
        SubstateValue::NonFungible(nf_container) => {
            let mut related_substates = vec![];

            // both the "data" and "mutable_data" fields are encoded as CBOR and could hold substate references
            if let Some(nf) = nf_container.contents() {
                let cbor_data = decode_into_cbor(nf.data())?;
                related_substates.append(&mut find_related_substates_in_cbor_value(&cbor_data)?);

                let cbor_mutable_data = decode_into_cbor(nf.mutable_data())?;
                related_substates.append(&mut find_related_substates_in_cbor_value(&cbor_mutable_data)?);
            }

            Ok(related_substates)
        },
        SubstateValue::NonFungibleIndex(index) => {
            // by definition a non fungible index always holds a reference to a non fungible substate
            let substate_address = SubstateAddress::NonFungible(index.referenced_address().clone());
            Ok(vec![substate_address])
        },
        // Other types of substates cannot hold references to other substates
        _ => Ok(vec![]),
    }
}

// recursively scan a CBOR value tree for substate references (represented as "tagged" values)
fn find_related_substates_in_cbor_value(value: &CborValue) -> Result<Vec<SubstateAddress>, anyhow::Error> {
    match value {
        ciborium::value::Value::Tag(tag, _) => {
            if let Some(tag_type) = BinaryTag::from_u64(*tag) {
                match tag_type {
                    BinaryTag::ComponentAddress => {
                        let component_address: ComponentAddress = value.deserialized()?;
                        let substate_address = SubstateAddress::Component(component_address);
                        return Ok(vec![substate_address]);
                    },
                    BinaryTag::NonFungibleAddress => {
                        let non_fungible_address: NonFungibleAddress = value.deserialized()?;
                        let substate_address = SubstateAddress::NonFungible(non_fungible_address);
                        return Ok(vec![substate_address]);
                    },
                    BinaryTag::ResourceAddress => {
                        let resource_address: ResourceAddress = value.deserialized()?;
                        let substate_address = SubstateAddress::Resource(resource_address);
                        return Ok(vec![substate_address]);
                    },
                    BinaryTag::VaultId => {
                        let vault_id: VaultId = value.deserialized()?;
                        let substate_address = SubstateAddress::Vault(vault_id);
                        return Ok(vec![substate_address]);
                    },
                    // other types of tags do not correspond with any type of addresses
                    _ => return Ok(vec![]),
                }
            }
            Ok(vec![])
        },
        ciborium::value::Value::Array(values) => {
            // recursively scan all the items in the array
            let related_substates = find_related_substates_in_cbor_values(values)?;
            Ok(related_substates)
        },
        ciborium::value::Value::Map(values) => {
            // recursively scan all fields in the map
            // we need to flatten the vec of value pairs into a vec of values, so it's easier to process
            let values: Vec<CborValue> = values.iter().flat_map(|(k, v)| vec![k.clone(), v.clone()]).collect();
            let related_substates = find_related_substates_in_cbor_values(&values)?;
            Ok(related_substates)
        },
        // other types are atomic so they will not contain any related substates
        _ => Ok(vec![]),
    }
}

fn find_related_substates_in_cbor_values(values: &[CborValue]) -> Result<Vec<SubstateAddress>, anyhow::Error> {
    let related_substates_per_item: Vec<Vec<SubstateAddress>> = values
        .iter()
        .map(find_related_substates_in_cbor_value)
        .collect::<Result<Vec<_>, _>>()?;
    let related_substates = related_substates_per_item.into_iter().flatten().collect();
    Ok(related_substates)
}

#[cfg(test)]
mod tests {
    use tari_common_types::types::PublicKey;
    use tari_crypto::commitment::HomomorphicCommitment;
    use tari_engine_types::{
        confidential::ConfidentialOutput,
        resource_container::ResourceContainer,
        substate::{Substate, SubstateValue},
        vault::Vault,
    };
    use tari_template_lib::{
        models::{EncryptedValue, VaultId},
        prelude::{Amount, ResourceAddress},
        Hash,
    };

    use crate::substate_decoder::decode_substate_into_json;

    #[test]
    fn it_decodes_confidential_vaults() {
        let address = ResourceAddress::new(Hash::default());

        let public_key = PublicKey::default();
        let confidential_output = ConfidentialOutput {
            commitment: HomomorphicCommitment::from_public_key(&public_key),
            stealth_public_nonce: Some(public_key.clone()),
            encrypted_value: Some(EncryptedValue([0; 24])),
            minimum_value_promise: 0,
        };
        let commitment = Some((public_key, confidential_output));

        let revealed_amount = Amount::zero();
        let container = ResourceContainer::confidential(address, commitment, revealed_amount);

        let vault_id = VaultId::new(Hash::default());
        let vault = Vault::new(vault_id, container);

        let substate_value = SubstateValue::Vault(vault);
        let substate = Substate::new(0, substate_value);

        assert!(decode_substate_into_json(&substate).is_ok());
    }
}
