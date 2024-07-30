//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::vec;

use tari_template_lib::{args, models::{Amount, ComponentAddress}};
use tari_template_test_tooling::TemplateTest;
use tari_transaction::{Instruction, Transaction};

#[test]
fn successful_assert() {
    let mut template_test = TemplateTest::new(Vec::<&str>::new());

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

    // Create user account to receive faucet tokens
    let (user_account, user_proof, user_key) = template_test.create_funded_account();
    let min_amount = Amount::new(1000);

    template_test.execute_expect_success(
        Transaction::builder()
            .call_method(faucet_component, "take_free_coins", args![])
            .put_last_instruction_output_on_workspace("free_coins")
            .assert_bucket_contains("free_coins", faucet_resource, min_amount)
            .call_method(user_account, "deposit", args![Workspace("free_coins")])
            .sign(&user_key)
            .build(),
        // Because we deny_all on deposits, we need to supply the owner proof to be able to deposit the initial
        // tokens into the new vaults
        vec![user_proof.clone()],
    );
}
