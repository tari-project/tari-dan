//  Copyright 2023. The Tari Project
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

use serde_json as json;
use tari_bor as cbor;

#[derive(Debug, thiserror::Error)]
pub enum JsonEncodingError {
    #[error("Could not decode the CBOR value: {0}")]
    BinaryEncoding(#[from] tari_bor::BorError),
    #[error("Serde error: {0}")]
    Serde(#[from] json::Error),
    #[error("Unexpected error: {0}")]
    Unexpected(String),
}

pub fn cbor_to_json(raw: &[u8]) -> Result<json::Value, JsonEncodingError> {
    let decoded_cbor: cbor::Value = tari_bor::decode(raw)?;
    let decoded_cbor = fix_invalid_object_keys(&decoded_cbor);
    let result = serde_json::to_value(decoded_cbor)?;

    Ok(result)
}

/// In JSON, all object keys must be string values.
/// But ciborium sometimes will use other types (e.g. Tags) as keys,
/// so in that case we transform the object into an array so it can be safely converted to JSON
/// AND we need to to it recursively
pub fn fix_invalid_object_keys(value: &cbor::Value) -> cbor::Value {
    match value {
        cbor::Value::Tag(tag, content) => {
            let fixed_content = fix_invalid_object_keys(content);
            cbor::Value::Tag(*tag, Box::new(fixed_content))
        },
        cbor::Value::Array(arr) => {
            let fixed_items = arr.iter().map(fix_invalid_object_keys).collect();
            cbor::Value::Array(fixed_items)
        },
        cbor::Value::Map(map) => {
            let has_invalid_keys = map.iter().any(|(k, _)| !k.is_text());

            if has_invalid_keys {
                let map_entries_as_arrays = map
                    .iter()
                    .map(|(k, v)| {
                        let fixed_key = fix_invalid_object_keys(k);
                        let fixed_value = fix_invalid_object_keys(v);
                        cbor::Value::Array(vec![fixed_key, fixed_value])
                    })
                    .collect();
                return cbor::Value::Array(map_entries_as_arrays);
            }

            let fixed_entries = map
                .iter()
                .map(|(k, v)| {
                    let fixed_value = fix_invalid_object_keys(v);
                    (k.to_owned(), fixed_value)
                })
                .collect();
            cbor::Value::Map(fixed_entries)
        },
        // other types are atomic and do not cause problems, so we just return them directly
        _ => value.to_owned(),
    }
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
        models::{Amount, ResourceAddress, VaultId},
        Hash,
    };

    use super::*;

    #[test]
    fn it_decodes_confidential_vaults() {
        let address = ResourceAddress::new(Hash::default());

        let public_key = PublicKey::default();
        let confidential_output = ConfidentialOutput {
            commitment: HomomorphicCommitment::from_public_key(&public_key),
            stealth_public_nonce: public_key.clone(),
            encrypted_data: Default::default(),
            minimum_value_promise: 0,
        };
        let commitment = Some((public_key, confidential_output));

        let revealed_amount = Amount::zero();
        let container = ResourceContainer::confidential(address, commitment, revealed_amount);

        let vault_id = VaultId::new(Hash::default());
        let vault = Vault::new(vault_id, container);

        let substate_value = SubstateValue::Vault(vault);
        let substate = Substate::new(0, substate_value);
        let substate_cbor = tari_bor::encode(&substate).unwrap();

        assert!(cbor_to_json(&substate_cbor).is_ok());
    }
}
