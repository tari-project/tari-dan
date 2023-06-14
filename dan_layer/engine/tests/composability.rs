//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::{
    commit_result::{ExecuteResult, RejectReason},
    instruction::Instruction,
};
use tari_template_lib::{
    args,
    models::{ComponentAddress, TemplateAddress},
};
use tari_template_test_tooling::TemplateTest;
use tari_transaction::Transaction;

fn setup() -> TemplateTest {
    TemplateTest::new(vec!["tests/templates/composability", "tests/templates/state"])
}

fn get_state_template_address(test: &TemplateTest) -> TemplateAddress {
    test.get_template_address("State")
}

fn get_composability_template_address(test: &TemplateTest) -> TemplateAddress {
    test.get_template_address("Composability")
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

#[test]
fn it_allows_function_to_function_calls() {
    let mut test = setup();
    let state_template = get_state_template_address(&test);
    let composability_template = get_composability_template_address(&test);

    // the composability template "new" function should create a new "state" component as well
    let res = test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: composability_template,
                function: "new".to_string(),
                args: args![state_template],
            }],
            vec![],
        )
        .unwrap();

    // extract the newly created component addresses
    let composability_component = extract_component_address_from_result(&res, "Composability");
    let state_component = extract_component_address_from_result(&res, "State");

    // the composability component exists in the network and is correctly initialized
    let inner_component_address: ComponentAddress =
        test.call_method(composability_component, "get_state_component_address", args![], vec![]);
    assert_eq!(inner_component_address, state_component);

    // the state component exists in the network and is correctly initialized
    let value: u32 = test.call_method(state_component, "get", args![], vec![]);
    assert_eq!(value, 0);
}

#[test]
fn it_allows_function_to_method_calls() {
    let mut test = setup();
    let state_template = get_state_template_address(&test);
    let composability_template = get_composability_template_address(&test);

    // the composability template "new" function should create a new "state" component as well
    let res = test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: composability_template,
                function: "new".to_string(),
                args: args![state_template],
            }],
            vec![],
        )
        .unwrap();

    // extract the newly created component addresses
    let composability_a_component = extract_component_address_from_result(&res, "Composability");
    let state_component = extract_component_address_from_result(&res, "State");

    // create a new composability component, this time using a constructor that gets information from a method call
    let res = test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: composability_template,
                function: "new_from_component".to_string(),
                args: args![composability_a_component],
            }],
            vec![],
        )
        .unwrap();
    let composability_b_component = extract_component_address_from_result(&res, "Composability");

    // the composability component exists in the network and is correctly initialized
    let inner_component_address: ComponentAddress = test.call_method(
        composability_b_component,
        "get_state_component_address",
        args![],
        vec![],
    );
    assert_eq!(inner_component_address, state_component);
}

#[test]
fn it_allows_method_to_method_calls() {
    let mut test = setup();
    let state_template = get_state_template_address(&test);
    let composability_template = get_composability_template_address(&test);

    // the composability template "new" function should create a new "state" component as well
    let res = test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: composability_template,
                function: "new".to_string(),
                args: args![state_template],
            }],
            vec![],
        )
        .unwrap();

    // extract the newly created component addresses
    let composability_component = extract_component_address_from_result(&res, "Composability");
    let state_component = extract_component_address_from_result(&res, "State");

    // the state component has an initial value of 0
    let value: u32 = test.call_method(state_component, "get", args![], vec![]);
    assert_eq!(value, 0);

    // perform the call to the composability component that will increase the counter
    test.call_method::<()>(
        composability_component,
        "increase_inner_state_component",
        args![],
        vec![],
    );

    // the state component has been increased
    let value: u32 = test.call_method(state_component, "get", args![], vec![]);
    assert_eq!(value, 1);
}

#[test]
fn it_allows_method_to_function_calls() {
    let mut test = setup();
    let state_template = get_state_template_address(&test);
    let composability_template = get_composability_template_address(&test);

    // the composability template "new" function should create a new "state" component as well
    let res = test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: composability_template,
                function: "new".to_string(),
                args: args![state_template],
            }],
            vec![],
        )
        .unwrap();

    // extract the newly created component addresses
    let composability_component = extract_component_address_from_result(&res, "Composability");
    let initial_state_component = extract_component_address_from_result(&res, "State");

    // perform the call to the composability component that will increase the counter
    test.call_method::<()>(
        composability_component,
        "increase_inner_state_component",
        args![],
        vec![],
    );
    let value: u32 = test.call_method(initial_state_component, "get", args![], vec![]);
    assert_eq!(value, 1);

    // perform the call to replace the inner state component for a new one
    test.call_method::<()>(
        composability_component,
        "replace_state_component",
        args![state_template],
        vec![],
    );

    // a new state component should have been initialized
    let new_state_component: ComponentAddress =
        test.call_method(composability_component, "get_state_component_address", args![], vec![]);
    assert_ne!(new_state_component, initial_state_component);
    let value: u32 = test.call_method(new_state_component, "get", args![], vec![]);
    assert_eq!(value, 0);
}

#[test]
fn it_fails_on_invalid_calls() {
    let mut test = setup();
    let (_, _, private_key) = test.create_owned_account();
    let state_template = get_state_template_address(&test);
    let composability_template = get_composability_template_address(&test);

    // the composability template "new" function should create a new "state" component as well
    let res = test
        .execute_and_commit(
            vec![Instruction::CallFunction {
                template_address: composability_template,
                function: "new".to_string(),
                args: args![state_template],
            }],
            vec![],
        )
        .unwrap();

    // extract the newly created component addresses
    let composability_component = extract_component_address_from_result(&res, "Composability");

    // the "invalid_state_call" method tries to call a non-existent method in the inner state component
    let result = test
        .try_execute_and_commit(
            Transaction::builder()
                .call_method(composability_component, "invalid_state_call", args![])
                .sign(&private_key)
                .build(),
            vec![],
        )
        .unwrap();
    let reason = result.expect_transaction_failure();
    result.expect_finalization_success();
    assert!(matches!(reason, RejectReason::ExecutionFailure(_)));
}
