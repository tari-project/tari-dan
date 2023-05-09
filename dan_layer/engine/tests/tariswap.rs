//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::instruction::Instruction;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress},
    prelude::ResourceAddress,
};
use tari_template_test_tooling::{SubstateType, TemplateTest};

struct TariSwapTest {
    template_test: TemplateTest,

    a_faucet: ComponentAddress,
    a_resource: ResourceAddress,

    b_faucet: ComponentAddress,
    b_resource: ResourceAddress,

    tariswap: ComponentAddress,
}

fn setup() -> TariSwapTest {
    let mut template_test = TemplateTest::new(vec!["tests/templates/tariswap", "tests/templates/faucet"]);

    // create the token pair for the swap pool
    let (a_faucet, a_resource) = create_faucet_component(&mut template_test, "A".to_string());
    let (b_faucet, b_resource) = create_faucet_component(&mut template_test, "B".to_string());

    let tariswap = create_tariswap_component(&mut template_test, a_faucet, b_faucet);

    TariSwapTest {
        template_test,
        a_faucet,
        a_resource,
        b_faucet,
        b_resource,
        tariswap,
    }
}

fn create_faucet_component(template_test: &mut TemplateTest, symbol: String) -> (ComponentAddress, ResourceAddress) {
    let initial_supply = Amount(1_000_000_000_000);
    let component_address: ComponentAddress =
        template_test.call_function("TestFaucet", "mint_with_symbol", args![initial_supply, symbol], vec![]);

    let resource_address = template_test
        .get_previous_output_address(SubstateType::Resource)
        .as_resource_address()
        .unwrap();

    (component_address, resource_address)
}

fn create_tariswap_component(
    template_test: &mut TemplateTest,
    a_faucet: ComponentAddress,
    b_faucet: ComponentAddress,
) -> ComponentAddress {
    let tariswap_template = template_test.get_template_address("TariSwapPool");

    template_test
        .execute_and_commit(
            vec![
                Instruction::CallMethod {
                    component_address: a_faucet,
                    method: "take_free_coins".to_string(),
                    args: args![],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"a_bucket".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: b_faucet,
                    method: "take_free_coins".to_string(),
                    args: args![],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"b_bucket".to_vec(),
                },
                Instruction::CallFunction {
                    template_address: tariswap_template,
                    function: "new".to_string(),
                    args: args![Variable("a_bucket"), Variable("b_bucket")],
                },
            ],
            vec![],
        )
        .unwrap();

    template_test
        .get_previous_output_address(SubstateType::Component)
        .as_component_address()
        .unwrap()
}

#[test]
fn swap() {
    let _tariswap_test = setup();
}
