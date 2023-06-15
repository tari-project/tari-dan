//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_core::consensus_constants::ConsensusConstants;
use tari_engine_types::{
    commit_result::{ExecuteResult, RejectReason},
    instruction::Instruction,
};
use tari_template_lib::{
    args,
    models::{ComponentAddress, TemplateAddress},
    prelude::{Amount, ResourceAddress},
};
use tari_template_test_tooling::TemplateTest;
use tari_transaction::Transaction;

struct ComposabilityTest {
    template_test: TemplateTest,
    composability_template: TemplateAddress,
    state_template: TemplateAddress,
}

struct ComposabilityComponentInfo {
    composability_component: ComponentAddress,
    state_component: ComponentAddress,
}

fn setup() -> ComposabilityTest {
    let template_test = TemplateTest::new(vec![
        "tests/templates/composability",
        "tests/templates/state",
        "tests/templates/faucet",
    ]);

    let composability_template = template_test.get_template_address("Composability");
    let state_template = template_test.get_template_address("State");

    ComposabilityTest {
        template_test,
        composability_template,
        state_template,
    }
}

fn initialize_composability(test: &mut ComposabilityTest) -> ComposabilityComponentInfo {
    // the composability template "new" function should create a new "state" component as well
    let res = test
        .template_test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: test.composability_template,
                function: "new".to_string(),
                args: args![test.state_template],
            }],
            vec![],
        )
        .unwrap();

    // extract the newly created component addresses
    let composability_component = extract_component_address_from_result(&res, "Composability");
    let state_component = extract_component_address_from_result(&res, "State");

    ComposabilityComponentInfo {
        composability_component,
        state_component,
    }
}

fn extract_component_address_from_result(result: &ExecuteResult, template_name: &str) -> ComponentAddress {
    let (substate_addr, _) = result
        .expect_success()
        .up_iter()
        .find(|(address, substate)| {
            address.is_component() && substate.substate_value().component().unwrap().module_name == template_name
        })
        .unwrap();
    substate_addr.as_component_address().unwrap()
}

fn create_resource_and_fund_account(test: &mut TemplateTest, account: ComponentAddress) -> ResourceAddress {
    // create a new fungible resource
    let faucet_template = test.get_template_address("TestFaucet");
    let initial_supply = Amount(1_000_000_000_000);
    let result = test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: faucet_template,
                function: "mint".to_string(),
                args: args![initial_supply],
            }],
            vec![],
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

    // take free coins into the account
    let _result = test
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
                    component_address: account,
                    method: "deposit".to_string(),
                    args: args![Variable("free_coins")],
                },
            ],
            vec![],
        )
        .unwrap();

    faucet_resource
}

#[test]
fn it_allows_function_to_function_calls() {
    let mut test = setup();
    let components = initialize_composability(&mut test);

    // the composability component exists in the network and is correctly initialized
    let inner_component_address: ComponentAddress = test.template_test.call_method(
        components.composability_component,
        "get_state_component_address",
        args![],
        vec![],
    );
    assert_eq!(inner_component_address, components.state_component);

    // the state component exists in the network and is correctly initialized
    let value: u32 = test
        .template_test
        .call_method(components.state_component, "get", args![], vec![]);
    assert_eq!(value, 0);
}

#[test]
fn it_allows_function_to_method_calls() {
    let mut test = setup();
    let components = initialize_composability(&mut test);
    let composability_component_0 = components.composability_component;

    // create a new composability component, this time using a constructor that gets information from a method call
    let res = test
        .template_test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: test.composability_template,
                function: "new_from_component".to_string(),
                args: args![composability_component_0],
            }],
            vec![],
        )
        .unwrap();
    let composability_component_1 = extract_component_address_from_result(&res, "Composability");

    // the composability component exists in the network and is correctly initialized
    let inner_component_address: ComponentAddress = test.template_test.call_method(
        composability_component_1,
        "get_state_component_address",
        args![],
        vec![],
    );
    assert_eq!(inner_component_address, components.state_component);
}

#[test]
fn it_allows_method_to_method_calls() {
    let mut test = setup();
    let components = initialize_composability(&mut test);

    // the state component has an initial value of 0
    let value: u32 = test
        .template_test
        .call_method(components.state_component, "get", args![], vec![]);
    assert_eq!(value, 0);

    // perform the call to the composability component that will increase the counter
    test.template_test.call_method::<()>(
        components.composability_component,
        "increase_inner_state_component",
        args![],
        vec![],
    );

    // the state component has been increased
    let value: u32 = test
        .template_test
        .call_method(components.state_component, "get", args![], vec![]);
    assert_eq!(value, 1);
}

#[test]
fn it_allows_method_to_function_calls() {
    let mut test = setup();
    let components = initialize_composability(&mut test);
    let initial_state_component = components.state_component;

    // perform the call to the composability component that will increase the counter
    test.template_test.call_method::<()>(
        components.composability_component,
        "increase_inner_state_component",
        args![],
        vec![],
    );
    let value: u32 = test
        .template_test
        .call_method(initial_state_component, "get", args![], vec![]);
    assert_eq!(value, 1);

    // perform the call to replace the inner state component for a new one
    test.template_test.call_method::<()>(
        components.composability_component,
        "replace_state_component",
        args![test.state_template],
        vec![],
    );

    // a new state component should have been initialized
    let new_state_component: ComponentAddress = test.template_test.call_method(
        components.composability_component,
        "get_state_component_address",
        args![],
        vec![],
    );
    assert_ne!(new_state_component, initial_state_component);
    let value: u32 = test
        .template_test
        .call_method(new_state_component, "get", args![], vec![]);
    assert_eq!(value, 0);
}

#[test]
fn it_fails_on_invalid_calls() {
    let mut test = setup();
    let components = initialize_composability(&mut test);
    let (_, _, private_key) = test.template_test.create_owned_account();

    // the "invalid_state_call" method tries to call a non-existent method in the inner state component
    let result = test
        .template_test
        .try_execute_and_commit(
            Transaction::builder()
                .call_method(components.composability_component, "invalid_state_call", args![])
                .sign(&private_key)
                .build(),
            vec![],
        )
        .unwrap();
    let reason = result.expect_transaction_failure();

    // TODO: inner errors are not properly propagated up, they all end up being "Engine call returned null for op
    // CallInvoke" we should be able to assert a more specific error cause
    assert!(matches!(reason, RejectReason::ExecutionFailure(_)));
}

#[test]
fn it_does_not_propagate_permissions() {
    let mut test = setup();
    let components = initialize_composability(&mut test);
    let (account, owner_proof, private_key) = test.template_test.create_owned_account();

    // create_resource_and_fund_account
    let fungible_resource = create_resource_and_fund_account(&mut test.template_test, account);

    // try to to an account withdraw inside the composability template, it should fail as the owner proof should not be
    // propagated
    let result = test
        .template_test
        .try_execute_and_commit(
            Transaction::builder()
                .call_method(components.composability_component, "malicious_withdraw", args![
                    account,
                    fungible_resource,
                    100
                ])
                .sign(&private_key)
                .build(),
            // note that we are actually passing a valid proof
            vec![owner_proof],
        )
        .unwrap();
    let reason = result.expect_transaction_failure();

    // TODO: inner errors are not properly propagated up, they all end up being "Engine call returned null for op
    // CallInvoke" we should be able to assert a more specific error cause
    assert!(matches!(reason, RejectReason::ExecutionFailure(_)));
}

#[test]
fn it_allows_multiple_recursion_levels() {
    let mut test = setup();

    // composability_0
    let mut components = initialize_composability(&mut test);
    let composability_0 = components.composability_component;

    // composability_1 has composability_0 nested
    components = initialize_composability(&mut test);
    let composability_1 = components.composability_component;
    test.template_test.call_method::<()>(
        composability_1,
        "set_nested_composability",
        args![composability_0],
        vec![],
    );

    // we have now: composability_1 -> composability_0 -> state
    // we want to access the innermost level from the outermost level
    let value: u32 = test
        .template_test
        .call_method(composability_1, "get_nested_value", args![], vec![]);
    assert_eq!(value, 0);
}

#[test]
fn it_fails_when_surpassing_recursion_limit() {
    let mut test = setup();
    let (_, _, private_key) = test.template_test.create_owned_account();
    let max_recursion_depth = ConsensusConstants::devnet().max_call_recursion_depth;

    // innermost composability component
    let mut components = initialize_composability(&mut test);
    let mut last_composability_component = components.composability_component;

    for _ in 0..max_recursion_depth {
        components = initialize_composability(&mut test);
        test.template_test.call_method::<()>(
            components.composability_component,
            "set_nested_composability",
            args![last_composability_component],
            vec![],
        );
        last_composability_component = components.composability_component;
    }

    // we have now nested more components than the recursion depth limit allows
    // se when we do a call that goes from the outermost to the innermost, it must fail
    let result = test
        .template_test
        .try_execute_and_commit(
            Transaction::builder()
                .call_method(last_composability_component, "get_nested_value", args![])
                .sign(&private_key)
                .build(),
            vec![],
        )
        .unwrap();
    let reason = result.expect_transaction_failure();

    // TODO: inner errors are not properly propagated up, they all end up being "Engine call returned null for op
    // CallInvoke" we should be able to assert a more specific error cause
    assert!(matches!(reason, RejectReason::ExecutionFailure(_)));
}
