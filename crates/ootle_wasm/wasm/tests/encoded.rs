//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! The decoders read values as the indexer serves them. Run with `wasm-pack test --node crates/ootle_wasm/wasm`.

#![cfg(target_arch = "wasm32")]

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::wasm_bindgen_test;

/// A fungible TARI vault holding 9577588913, from `ootle_sdk_core/fixtures/substate_decode/fungible_vault.json`.
const SERVED_VAULT: &str = "ggKBgoIAg9iDWCAiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIhsAAAACOt5osQAA";

#[wasm_bindgen_test]
fn a_served_substate_value_decodes_to_json() {
    let json = ootle_wasm::decode_substate_value(SERVED_VAULT).unwrap();

    assert!(json.starts_with(r#"{"Vault":"#), "{json}");
    assert!(json.contains(r#""amount":"9577588913""#), "{json}");
}

#[wasm_bindgen_test]
fn a_value_that_does_not_decode_throws() {
    let Err(err) = ootle_wasm::decode_execute_result(SERVED_VAULT) else {
        panic!("a substate value decoded as an execute result");
    };
    let message = String::from(JsValue::from(err).unchecked_into::<js_sys::Error>().message());
    assert!(message.starts_with("The execute result does not decode"), "{message}");
}
