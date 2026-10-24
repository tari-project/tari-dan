//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Decoders for the engine values the indexer serves in their CBOR encoding, base64 in JSON.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use tari_engine_types::{commit_result::ExecuteResult, encoded::Encoded, substate::SubstateValue};

use crate::error::OotleWasmError;

/// The `SubstateValue` that `encoded` holds, as JSON.
pub fn decode_substate_value_json(encoded: &str) -> Result<String, OotleWasmError> {
    let value = Encoded::<SubstateValue>::from_bytes(base64_bytes(encoded)?)
        .decode()
        .map_err(|e| OotleWasmError::UndecodableValue {
            what: "substate value",
            details: e.to_string(),
        })?;
    serde_json::to_string(&value).map_err(OotleWasmError::JsonSerialize)
}

/// The `ExecuteResult` that `encoded` holds, as JSON.
pub fn decode_execute_result_json(encoded: &str) -> Result<String, OotleWasmError> {
    let value = Encoded::<ExecuteResult>::from_bytes(base64_bytes(encoded)?)
        .decode()
        .map_err(|e| OotleWasmError::UndecodableValue {
            what: "execute result",
            details: e.to_string(),
        })?;
    serde_json::to_string(&value).map_err(OotleWasmError::JsonSerialize)
}

fn base64_bytes(encoded: &str) -> Result<Vec<u8>, OotleWasmError> {
    STANDARD
        .decode(encoded)
        .map_err(|e| OotleWasmError::InvalidBase64(e.to_string()))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tari_engine_types::{
        commit_result::{FinalizeResult, TransactionResult},
        fees::FeeReceipt,
        resource_container::ResourceContainer,
        substate::SubstateDiff,
        vault::Vault,
    };
    use tari_template_lib_types::{Hash32, constants::TARI_TOKEN};

    use super::*;

    fn served<T>(encoded: Encoded<T>) -> String {
        STANDARD.encode(encoded.as_bytes())
    }

    #[test]
    fn a_served_substate_value_decodes_to_its_serde_json() {
        let value = SubstateValue::Vault(Vault::new(ResourceContainer::public_fungible(TARI_TOKEN, 100u64)));

        let json = decode_substate_value_json(&served(Encoded::encode(&value).unwrap())).unwrap();

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
            serde_json::to_value(&value).unwrap()
        );
    }

    #[test]
    fn a_served_execute_result_decodes_to_its_serde_json() {
        let value = ExecuteResult {
            finalize: FinalizeResult::new(
                Hash32::from_array([7; 32]),
                vec![],
                vec![],
                TransactionResult::Accept(SubstateDiff::new()),
                FeeReceipt::default(),
            ),
            execution_time: Duration::from_secs(1),
            execute_epoch: None,
            wasm_execution_points: 3,
            native_execution_points: 4,
        };

        let json = decode_execute_result_json(&served(Encoded::encode(&value).unwrap())).unwrap();

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
            serde_json::to_value(&value).unwrap()
        );
    }

    #[test]
    fn bytes_that_are_not_base64_are_rejected() {
        assert!(matches!(
            decode_substate_value_json("not base64!"),
            Err(OotleWasmError::InvalidBase64(_))
        ));
    }

    #[test]
    fn bytes_that_do_not_decode_are_rejected() {
        let value = SubstateValue::Vault(Vault::new(ResourceContainer::public_fungible(TARI_TOKEN, 100u64)));
        let mut bytes = Encoded::encode(&value).unwrap().into_bytes();
        bytes.push(0);

        assert!(matches!(
            decode_substate_value_json(&STANDARD.encode(bytes)),
            Err(OotleWasmError::UndecodableValue { .. })
        ));
        assert!(matches!(
            decode_execute_result_json(&served(Encoded::encode(&value).unwrap())),
            Err(OotleWasmError::UndecodableValue { .. })
        ));
    }
}
