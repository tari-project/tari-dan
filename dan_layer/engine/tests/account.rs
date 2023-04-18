//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::instruction::Instruction;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress},
};
use tari_template_test_tooling::TemplateTest;

#[test]
fn basic_faucet_transfer() {
    let mut template_test = TemplateTest::new(vec!["tests/templates/faucet"]);

    let faucet_template = template_test.get_template_address("TestFaucet");

    let initial_supply = Amount(1_000_000_000_000);
    let result = template_test
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

    // Create sender and receiver accounts
    let (sender_address, sender_proof, _) = template_test.create_owned_account();
    let (receiver_address, _, _) = template_test.create_owned_account();

    let _result = template_test
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
            vec![],
        )
        .unwrap();

    let result = template_test
        .execute_and_commit(
            vec![
                Instruction::CallMethod {
                    component_address: sender_address,
                    method: "withdraw".to_string(),
                    args: args![faucet_resource, Amount(100)],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"foo_bucket".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: receiver_address,
                    method: "deposit".to_string(),
                    args: args![Variable("foo_bucket")],
                },
                Instruction::CallMethod {
                    component_address: sender_address,
                    method: "balance".to_string(),
                    args: args![faucet_resource],
                },
                Instruction::CallMethod {
                    component_address: receiver_address,
                    method: "balance".to_string(),
                    args: args![faucet_resource],
                },
            ],
            // Sender proof needed to withdraw
            vec![sender_proof],
        )
        .unwrap();

    assert_eq!(result.finalize.execution_results[3].decode::<Amount>().unwrap(), 900);
    assert_eq!(result.finalize.execution_results[4].decode::<Amount>().unwrap(), 100);
}

#[test]
fn withdraw_from_account_prevented() {
    let mut template_test = TemplateTest::new(vec!["tests/templates/faucet"]);

    let faucet_template = template_test.get_template_address("TestFaucet");

    let initial_supply = Amount(1_000_000_000_000);
    let result = template_test
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
        .find_map(|(addr, _)| addr.as_resource_address())
        .unwrap();

    // Create sender and receiver accounts
    let (source_account, _, _) = template_test.create_owned_account();

    let _result = template_test
        .execute_and_commit_manifest(
            r#"
                let source_account = var!["source_account"];
                let faucet_component = var!["faucet_component"];
                
                let free_coins = faucet_component.take_free_coins();
                source_account.deposit(free_coins);
            "#,
            [
                ("source_account", source_account.into()),
                ("faucet_component", faucet_component.into()),
            ],
            vec![],
        )
        .unwrap();

    let (dest_address, non_owning_token, _) = template_test.create_owned_account();

    let err = template_test
        .execute_and_commit_manifest(
            r#"
                let source_account = var!["source_account"];
                let dest_account = var!["dest_account"];
                let resource = var!["resource"];
                let stolen_coins = source_account.withdraw(resource, Amount::new(100));
                dest_account.deposit(stolen_coins);
            "#,
            [
                ("source_account", source_account.into()),
                ("dest_account", dest_address.into()),
                ("resource", faucet_resource.into()),
            ],
            // VNs provide the token that signed the transaction, which in this case is the non_owning_token
            vec![non_owning_token],
        )
        .unwrap_err();

    assert!(err.to_string().contains("Access Denied: template.Account.withdraw"));

    let result = template_test
        .execute_and_commit_manifest(
            r#"
                let dest_account = var!["dest_account"];
                let resource = var!["resource"];
                dest_account.balance(resource);
            "#,
            [
                ("dest_account", dest_address.into()),
                ("resource", faucet_resource.into()),
            ],
            // Nothing required for balance check at the moment
            vec![],
        )
        .unwrap();
    assert_eq!(
        result.finalize.execution_results[0].decode::<Amount>().unwrap(),
        Amount(0)
    );
}
