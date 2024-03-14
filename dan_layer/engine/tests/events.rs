//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_engine::runtime::RuntimeError;
use tari_engine_types::instruction::Instruction;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress},
};
use tari_template_test_tooling::{support::assert_error::assert_reject_reason, TemplateTest};
use tari_transaction::Transaction;

#[test]
fn basic_emit_event() {
    let mut template_test = TemplateTest::new(vec!["tests/templates/events"]);
    let event_emitter_template = template_test.get_template_address("EventEmitter");
    let topic = "Hello world !";
    let result = template_test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: event_emitter_template,
                function: "test_function".to_string(),
                args: args![topic],
            }],
            vec![],
        )
        .expect("Failed to emit test event");
    assert!(result.finalize.is_accept());
    assert_eq!(result.finalize.events.len(), 1);
    assert_eq!(result.finalize.events[0].topic(), topic);
    assert_eq!(result.finalize.events[0].template_address(), event_emitter_template);
    assert_eq!(result.finalize.events[0].component_address(), None);
    assert_eq!(
        result.finalize.events[0].get_payload("my").unwrap(),
        "event".to_string()
    );
}

#[test]
fn cannot_use_standard_topic() {
    let mut template_test = TemplateTest::new(vec!["tests/templates/events"]);
    let event_emitter_template = template_test.get_template_address("EventEmitter");
    let (_, _, private_key) = template_test.create_owned_account();
    let invalid_topic = "std.mytopic";
    let reason = template_test.execute_expect_failure(
        Transaction::builder()
            .call_function(event_emitter_template, "test_function", args![invalid_topic])
            .sign(&private_key)
            .build(),
        [].into(),
    );
    assert_reject_reason(reason, RuntimeError::InvalidEventTopic {
        topic: invalid_topic.to_owned(),
    });
}

#[test]
fn builtin_vault_events() {
    let mut template_test = TemplateTest::new(Vec::<&str>::new());

    // create a fungible resource for transfer
    let faucet_template = template_test.get_template_address("TestFaucet");
    let initial_supply = Amount(1_000_000_000_000);
    let result = template_test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: faucet_template,
                function: "mint".to_string(),
                args: args![initial_supply],
            }],
            vec![template_test.get_test_proof()],
        )
        .unwrap();
    let faucet_component: ComponentAddress = result.finalize.execution_results[0].decode().unwrap();
    let faucet_resource = result
        .finalize
        .result
        .expect("Faucet mint failed")
        .up_iter()
        .find_map(|(address, _)| address.as_resource_address())
        .unwrap();

    // Create sender and receiver accounts
    let (sender_address, sender_proof, _) = template_test.create_owned_account();
    let (receiver_address, _, _) = template_test.create_owned_account();
    template_test
        .execute_and_commit(
            vec![
                Instruction::CallMethod {
                    component_address: faucet_component,
                    method: "take_free_coins".to_string(),
                    args: args![],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"free_coins".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: sender_address,
                    method: "deposit".to_string(),
                    args: args![Variable("free_coins")],
                },
            ],
            vec![template_test.get_test_proof()],
        )
        .unwrap();

    // transfer some tokens between accounts
    let amount = Amount(100);
    let result = template_test
        .execute_and_commit(
            vec![
                Instruction::CallMethod {
                    component_address: sender_address,
                    method: "withdraw".to_string(),
                    args: args![faucet_resource, amount],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"foo_bucket".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: receiver_address,
                    method: "deposit".to_string(),
                    args: args![Variable("foo_bucket")],
                },
            ],
            // Sender proof needed to withdraw
            vec![sender_proof],
        )
        .unwrap();

    assert!(result.finalize.is_accept());

    // a standard event for the withdraw must have been emmitted
    assert!(result.finalize.events.iter().any(|e| {
        e.topic() == "std.vault.withdraw".to_owned() &&
            e.template_address() == ACCOUNT_TEMPLATE_ADDRESS &&
            e.component_address().unwrap() == sender_address &&
            *e.payload().get("resource_address").unwrap() == faucet_resource.to_string() &&
            *e.payload().get("amount").unwrap() == amount.to_string()
    }));

    // a standard event for the deposit must have been emmitted
    assert!(result.finalize.events.iter().any(|e| {
        e.topic() == "std.vault.deposit".to_owned() &&
            e.template_address() == ACCOUNT_TEMPLATE_ADDRESS &&
            e.component_address().unwrap() == receiver_address &&
            *e.payload().get("resource_address").unwrap() == faucet_resource.to_string() &&
            *e.payload().get("amount").unwrap() == amount.to_string()
    }));
}
