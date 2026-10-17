//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Amount arguments must reach the stealth exports exactly as the JS caller passed them. Run with
//! `wasm-pack test --node crates/ootle_wasm/wasm`.

#![cfg(target_arch = "wasm32")]

use js_sys::BigInt;
use wasm_bindgen::{JsCast, JsError, JsValue};
use wasm_bindgen_test::wasm_bindgen_test;

/// 2^64, 2^64 + 5, 2^65 and -1: each one is a valid `u64` once reduced modulo 2^64.
const OUT_OF_RANGE: [&str; 4] = [
    "18446744073709551616",
    "18446744073709551621",
    "36893488147419103232",
    "-1",
];

fn bigint(value: &str) -> BigInt {
    BigInt::new(&JsValue::from_str(value)).unwrap()
}

fn one() -> BigInt {
    BigInt::from(1u64)
}

fn assert_rejected<T>(param: &str, value: &str, result: Result<T, JsError>) {
    let Err(err) = result else {
        panic!("{param} = {value} was accepted");
    };
    let message = String::from(JsValue::from(err).unchecked_into::<js_sys::Error>().message());
    assert!(
        message.starts_with(&format!("{param} must be")),
        "{param} = {value} failed with an unrelated error: {message}"
    );
}

#[wasm_bindgen_test]
fn stealth_inputs_statement_rejects_out_of_range_revealed_amount() {
    for value in OUT_OF_RANGE {
        assert_rejected(
            "revealed_amount_microtari",
            value,
            ootle_wasm::build_stealth_inputs_statement(&[], bigint(value)),
        );
        assert_rejected(
            "revealed_amount_microtari",
            value,
            ootle_wasm::build_stealth_inputs_statement_from_inputs("[]", bigint(value)),
        );
    }
}

#[wasm_bindgen_test]
fn stealth_outputs_statement_rejects_out_of_range_revealed_amount() {
    for value in OUT_OF_RANGE {
        assert_rejected(
            "revealed_output_amount_microtari",
            value,
            ootle_wasm::generate_stealth_outputs_statement("[]", bigint(value), &[]),
        );
    }
}

#[wasm_bindgen_test]
fn stealth_output_witness_rejects_out_of_range_amounts() {
    let witness = |amount: BigInt, minimum_value_promise: BigInt| {
        ootle_wasm::create_stealth_output_witness(
            0x10,
            &[0; 32],
            &[0; 32],
            amount,
            "resource_0000000000000000000000000000000000000000000000000000000000000000",
            None,
            None,
            None,
            minimum_value_promise,
        )
    };
    for value in OUT_OF_RANGE {
        assert_rejected("amount", value, witness(bigint(value), BigInt::from(0u64)));
        assert_rejected("minimum_value_promise", value, witness(one(), bigint(value)));
    }
}

#[wasm_bindgen_test]
fn stealth_transfer_statement_rejects_out_of_range_revealed_amounts() {
    for value in OUT_OF_RANGE {
        assert_rejected(
            "revealed_input_amount_microtari",
            value,
            ootle_wasm::build_stealth_transfer_statement("[]", bigint(value), "[]", one(), &[]),
        );
        assert_rejected(
            "revealed_output_amount_microtari",
            value,
            ootle_wasm::build_stealth_transfer_statement("[]", one(), "[]", bigint(value), &[]),
        );
    }
}

#[wasm_bindgen_test]
fn viewable_balance_rejects_out_of_range_amounts() {
    for value in OUT_OF_RANGE {
        assert_rejected(
            "amount",
            value,
            ootle_wasm::generate_elgamal_viewable_balance_proof(&[0; 32], bigint(value), &[0; 32], &[0; 32]),
        );
        assert_rejected(
            "min_value",
            value,
            ootle_wasm::decrypt_elgamal_viewable_balance("{}", &[], &[], &[], bigint(value), one()),
        );
        assert_rejected(
            "max_value",
            value,
            ootle_wasm::decrypt_elgamal_viewable_balance("{}", &[], &[], &[], one(), bigint(value)),
        );
    }
}

#[wasm_bindgen_test]
fn amount_given_as_a_js_number_is_rejected() {
    let number = JsValue::from_f64(5.0).unchecked_into::<BigInt>();
    assert_rejected(
        "revealed_amount_microtari",
        "5 (number)",
        ootle_wasm::build_stealth_inputs_statement(&[], number),
    );
}

#[wasm_bindgen_test]
fn in_range_revealed_amounts_are_preserved() {
    for value in ["0", "1", "5", "18446744073709551615"] {
        let statement = ootle_wasm::build_stealth_inputs_statement(&[], bigint(value))
            .unwrap_or_else(|_| panic!("revealed amount {value} was rejected"));
        assert!(
            statement.contains(&format!("\"revealed_amount\":\"{value}\"")),
            "revealed amount {value} was not preserved: {statement}"
        );
    }
}
