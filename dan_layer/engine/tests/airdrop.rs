//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::iter;

use tari_engine_types::{instruction::Instruction, substate::SubstateAddress};
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress},
};
use tari_template_test_tooling::{SubstateType, TemplateTest};

fn setup() -> (TemplateTest, ComponentAddress, SubstateAddress) {
    let mut template_test = TemplateTest::new(vec!["tests/templates/nft/airdrop"]);

    let airdrop: ComponentAddress = template_test.call_function("Airdrop", "new", args![], vec![]);

    let airdrop_resx = template_test.get_previous_output_address(SubstateType::Resource);

    (template_test, airdrop, airdrop_resx)
}

#[test]
fn airdrop() {
    let (mut template_test, airdrop, airdrop_resx) = setup();

    let total_supply: Amount = template_test.call_method(airdrop, "total_supply", args![], vec![]);
    assert_eq!(total_supply, Amount(10));

    // Create 2 accounts
    let (owner_proof, _) = template_test.create_owner_proof();
    let account_template_addr = template_test.get_template_address("Account");
    let result = template_test
        .execute_and_commit(
            iter::repeat_with(|| Instruction::CallFunction {
                template_address: account_template_addr,
                function: "create".to_string(),
                args: args![owner_proof.clone()],
            })
            .take(2)
            .collect(),
            vec![],
        )
        .unwrap();

    let addresses = result
        .execution_results
        .iter()
        .map(|r| r.decode::<ComponentAddress>().unwrap())
        .collect::<Vec<_>>();

    template_test.call_method::<()>(airdrop, "open_airdrop", args![], vec![]);

    template_test
        .execute_and_commit(
            addresses
                .iter()
                .flat_map(|addr| {
                    [Instruction::CallMethod {
                        component_address: airdrop,
                        method: "add_recipient".to_string(),
                        args: args![addr],
                    }]
                })
                .collect(),
            vec![],
        )
        .unwrap();

    let instructions = addresses
        .into_iter()
        .flat_map(|addr| {
            [
                Instruction::CallMethod {
                    component_address: airdrop,
                    method: "claim_any".to_string(),
                    args: args![addr],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"claimed".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: addr,
                    method: "deposit".to_string(),
                    args: args![Variable("claimed")],
                },
                Instruction::CallMethod {
                    component_address: addr,
                    method: "balance".to_string(),
                    args: args![airdrop_resx.as_resource_address().unwrap()],
                },
            ]
        })
        .collect();
    let result = template_test.execute_and_commit(instructions, vec![]).unwrap();

    for i in 0..10 {
        assert_eq!(
            result.execution_results[3 + (i * 4)].decode::<Amount>().unwrap(),
            Amount(1)
        );
    }
}
