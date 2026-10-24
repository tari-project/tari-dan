//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Decoders for the engine values the indexer serves in their CBOR encoding, base64 in JSON. The module exports only
//! these, which keeps it small enough for a web page to load.

use wasm_bindgen::prelude::*;

/// Decode a `SubstateValue` as the indexer serves it (its CBOR encoding, base64) → JSON string.
#[wasm_bindgen(js_name = "decodeSubstateValue")]
pub fn decode_substate_value(encoded: &str) -> Result<String, JsError> {
    ootle_wasm_core::encoded::decode_substate_value_json(encoded).map_err(|e| JsError::new(&e.to_string()))
}

/// Decode an `ExecuteResult` as the indexer serves it (its CBOR encoding, base64) → JSON string.
#[wasm_bindgen(js_name = "decodeExecuteResult")]
pub fn decode_execute_result(encoded: &str) -> Result<String, JsError> {
    ootle_wasm_core::encoded::decode_execute_result_json(encoded).map_err(|e| JsError::new(&e.to_string()))
}

/// Decode a `TransactionReceipt` as the indexer serves it (its CBOR encoding, base64) → JSON string.
#[wasm_bindgen(js_name = "decodeTransactionReceipt")]
pub fn decode_transaction_receipt(encoded: &str) -> Result<String, JsError> {
    ootle_wasm_core::encoded::decode_transaction_receipt_json(encoded).map_err(|e| JsError::new(&e.to_string()))
}

/// Decode an `Event` as the indexer serves it (its CBOR encoding, base64) → JSON string.
#[wasm_bindgen(js_name = "decodeEvent")]
pub fn decode_event(encoded: &str) -> Result<String, JsError> {
    ootle_wasm_core::encoded::decode_event_json(encoded).map_err(|e| JsError::new(&e.to_string()))
}

/// Decode a `Resource` as the indexer serves it (its CBOR encoding, base64) → JSON string.
#[wasm_bindgen(js_name = "decodeResource")]
pub fn decode_resource(encoded: &str) -> Result<String, JsError> {
    ootle_wasm_core::encoded::decode_resource_json(encoded).map_err(|e| JsError::new(&e.to_string()))
}

/// Decode a `Utxo` as the indexer serves it (its CBOR encoding, base64) → JSON string.
#[wasm_bindgen(js_name = "decodeUtxo")]
pub fn decode_utxo(encoded: &str) -> Result<String, JsError> {
    ootle_wasm_core::encoded::decode_utxo_json(encoded).map_err(|e| JsError::new(&e.to_string()))
}
