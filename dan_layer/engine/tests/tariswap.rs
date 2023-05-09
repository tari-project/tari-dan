//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

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
}

fn setup() -> TariSwapTest {
    let mut template_test = TemplateTest::new(vec!["tests/templates/tariswap", "tests/templates/faucet"]);

    // create the token pair for the swap pool
    let (a_faucet, a_resource) = create_faucet_component(&mut template_test);
    let (b_faucet, b_resource) = create_faucet_component(&mut template_test);

    TariSwapTest {
        template_test,
        a_faucet,
        a_resource,
        b_faucet,
        b_resource,
    }
}

fn create_faucet_component(template_test: &mut TemplateTest) -> (ComponentAddress, ResourceAddress) {
    let initial_supply = Amount(1_000_000_000_000);
    let component_address: ComponentAddress =
        template_test.call_function("TestFaucet", "mint", args![initial_supply], vec![]);

    let resource_address = template_test
        .get_previous_output_address(SubstateType::Resource)
        .as_resource_address()
        .unwrap();

    (component_address, resource_address)
}

#[test]
fn swap() {
    let _tariswap_test = setup();
}
