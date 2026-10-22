//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::RawCbor;
use tari_engine::runtime::RuntimeError;
use tari_engine_types::hashing::hash_template_code;
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_abi::{EngineOp, FunctionDef, TemplateDef, TemplateDefV1, Type};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args::EmitEventArg,
    types::{Amount, Metadata, TemplateAddress, bytes::Bytes},
};
use tari_template_test_tooling::{
    Package,
    TemplateTest,
    support::assert_error::assert_reject_reason,
    template_lib_types::constants::TARI_TOKEN,
};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn basic_emit_event() {
    let mut template_test = TemplateTest::new(CRATE_PATH, vec!["tests/templates/events"]);
    let event_emitter_template = template_test.get_template_address("EventEmitter");
    let topic = "Hello_world";
    let result = template_test.execute_expect_success(
        template_test
            .transaction()
            .call_function(event_emitter_template, "test_function", args![topic])
            .build_and_seal(template_test.secret_key()),
        vec![],
    );
    assert!(result.finalize.is_accept());
    assert_eq!(result.finalize.events.len(), 1);
    assert_eq!(result.finalize.events[0].topic(), format!("EventEmitter.{}", topic));
    assert_eq!(*result.finalize.events[0].template_address(), event_emitter_template);
    assert_eq!(result.finalize.events[0].substate_id(), None);
    assert_eq!(result.finalize.events[0].payload().get_str("my"), Some("event"));

    template_test.assert_substate_lengths_match_encoding();
}

#[test]
fn cannot_use_standard_topic() {
    let mut template_test = TemplateTest::new(CRATE_PATH, vec!["tests/templates/events"]);
    let event_emitter_template = template_test.get_template_address("EventEmitter");
    let (_, _, private_key) = template_test.create_funded_account();
    let invalid_topic = "std.mytopic";
    let reason = template_test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(event_emitter_template, "test_function", args![invalid_topic])
            .build_and_seal(&private_key),
        [].into(),
    );
    assert_reject_reason(reason, RuntimeError::InvalidEventTopic {
        topic: invalid_topic.to_owned(),
        reason: "topics starting with 'std.' are reserved for standard events".to_owned(),
    });
}

/// Renders bytes as a WAT string literal.
fn wat_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

/// A hand-written template named `template_name` whose `main` function emits `topic` with an empty
/// payload. The `#[template]` macro only produces Rust identifiers as template names, so any other
/// name needs a module built by hand.
fn event_emitting_template(template_name: &str, topic: &str) -> Vec<u8> {
    let template_def = TemplateDef::V1(TemplateDefV1 {
        template_name: template_name.to_string(),
        abi_version: 0,
        functions: vec![FunctionDef {
            name: "main".to_string(),
            arguments: vec![],
            output: Type::Unit,
            is_mut: false,
            is_migration: false,
        }],
    })
    .encode_for_wasm_embedding()
    .unwrap();
    let arg = tari_bor::encode(&EmitEventArg {
        topic: topic.to_string(),
        payload: Metadata::new(),
    })
    .unwrap();

    // Functions are referenced by index: a named symbol makes `wat` emit a `name` custom section,
    // which the loader rejects. `tari_alloc` hands out one fixed region at offset 1024, `main`
    // returns the encoded unit at offset 16 and the event argument sits at offset 2048.
    let wat = format!(
        r#"
        (module
          (import "env" "tari_engine" (func (param i32 i32 i32) (result i32)))
          (memory (export "memory") 3)
          (data (i32.const 16) "\05\00\00\00\80")
          (data (i32.const 2048) "{arg}")
          (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "tari_free") (param i32))
          (func (export "{template_name}_main") (param i32 i32) (result i32)
            (drop (call 0 (i32.const {op}) (i32.const 2048) (i32.const {arg_len})))
            (i32.const 20))
          (@custom "tari_tdef" "{template_def}")
        )
        "#,
        arg = wat_bytes(&arg),
        arg_len = arg.len(),
        op = EngineOp::EmitEvent.as_i32(),
        template_def = wat_bytes(&template_def),
    );
    wat::parse_str(&wat).unwrap()
}

/// Registers `code` as a template and returns a harness that can call it.
fn test_with_template(code: Vec<u8>) -> (TemplateTest, TemplateAddress) {
    let address = hash_template_code(&code);
    let mut builder = Package::builder();
    builder.add_all_builtin_templates();
    builder
        .add_template_from_code(address, code)
        .expect("template was rejected by the loader");
    let mut test = TemplateTest::from_package(builder.build());
    test.bootstrap_state();
    (test, address)
}

#[test]
fn a_template_name_cannot_produce_a_standard_topic() {
    let cases = [
        ("std.vault", "deposit", "std.vault.deposit"),
        ("std", "vault.deposit", "std.vault.deposit"),
    ];
    for (template_name, topic, emitted_topic) in cases {
        let (mut test, template) = test_with_template(event_emitting_template(template_name, topic));
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_function(template, "main", args![])
                .build_and_seal(test.secret_key()),
            vec![],
        );
        assert_reject_reason(reason, RuntimeError::InvalidEventTopic {
            topic: emitted_topic.to_owned(),
            reason: "topics starting with 'std.' are reserved for standard events".to_owned(),
        });
    }
}

#[test]
fn a_hand_written_template_emits_a_custom_event() {
    let (mut test, template) = test_with_template(event_emitting_template("Emitter", "hello.world"));
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template, "main", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    assert_eq!(result.finalize.events.len(), 1);
    assert_eq!(result.finalize.events[0].topic(), "Emitter.hello.world");
    assert_eq!(*result.finalize.events[0].template_address(), template);
}

#[test]
fn builtin_vault_events() {
    let mut test = TemplateTest::new(CRATE_PATH, Vec::<&str>::new());

    // Create sender and receiver accounts
    let (sender_address, sender_proof, _) = test.create_funded_account();
    let (receiver_address, _, _) = test.create_empty_account();

    // transfer some tokens between accounts
    let amount = Amount::from(100u64);
    let result = test.build_and_execute(
        Transaction::builder_localnet(Epoch(1))
            .call_method(sender_address, "withdraw", args![TARI_TOKEN, amount])
            .put_last_instruction_output_on_workspace("foo_bucket")
            .call_method(receiver_address, "deposit", args![Workspace("foo_bucket")]),
        // Sender proof needed to withdraw
        vec![sender_proof],
    );
    result.expect_success();

    // a standard event for the withdraw must have been emmitted
    let event = result
        .finalize
        .events
        .iter()
        .find(|e| e.topic() == "std.vault.withdraw")
        .unwrap();
    assert_eq!(*event.template_address(), ACCOUNT_TEMPLATE_ADDRESS);
    // assert_eq!(event.component_address().unwrap(), sender_address);
    // The vault is what identifies the transfer; its resource is read off the vault substate.
    assert!(event.substate_id().unwrap().is_vault());
    assert!(event.payload().get("resource_address").is_none());
    assert_eq!(event.payload().get_as::<Amount>("amount").unwrap(), Some(amount));

    // a standard event for the deposit must have been emmitted
    let event = result
        .finalize
        .events
        .iter()
        .find(|e| e.topic() == "std.vault.deposit")
        .unwrap();
    assert_eq!(*event.template_address(), ACCOUNT_TEMPLATE_ADDRESS);
    // assert_eq!(event.component_address().unwrap(), receiver_address);
    assert!(event.substate_id().unwrap().is_vault());
    assert!(event.payload().get("resource_address").is_none());
    assert_eq!(event.payload().get_as::<Amount>("amount").unwrap(), Some(amount));
}

/// One well-formed CBOR item each, with no `tari_bor::Value` form and so no JSON form: a simple
/// value, and an array nested past `tari_bor::MAX_DECODE_DEPTH`. A transaction argument cannot carry
/// either, so the templates build them.
fn unrepresentable_values() -> Vec<(&'static str, Vec<u8>)> {
    let mut deep = vec![0x81; tari_bor::MAX_DECODE_DEPTH + 6];
    deep.push(0x00);
    let values = vec![("simple", vec![0xe0]), ("deep", deep)];
    for (name, bytes) in &values {
        let mut metadata = Metadata::new();
        metadata.insert_raw("value", tari_bor::decode_exact::<RawCbor>(bytes).unwrap());
        // Were one of these to commit, every reader serving it as JSON would fail on it.
        assert!(serde_json::to_value(&metadata).is_err(), "{name} has a JSON form");
    }
    values
}

#[test]
fn an_event_payload_value_must_be_representable() {
    let mut test = TemplateTest::new(CRATE_PATH, vec!["tests/templates/events"]);
    let template = test.get_template_address("EventEmitter");
    let (_, _, key) = test.create_funded_account();

    for (name, bytes) in unrepresentable_values() {
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_function(template, "emit_raw_payload_value", args![Bytes::from(bytes)])
                .build_and_seal(&key),
            vec![],
        );
        assert!(
            reason.to_string().contains("not a representable CBOR value"),
            "{name}: {reason}"
        );
    }

    let representable = Bytes::from(tari_bor::encode(&1u32).unwrap());
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template, "emit_raw_payload_value", args![representable])
            .build_and_seal(&key),
        vec![],
    );
}
