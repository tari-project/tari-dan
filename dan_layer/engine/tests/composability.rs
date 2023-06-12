//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::{commit_result::ExecuteResult, instruction::Instruction};
use tari_template_lib::{
    args,
    models::{ComponentAddress, TemplateAddress},
};
use tari_template_test_tooling::TemplateTest;

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
