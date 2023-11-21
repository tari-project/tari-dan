//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_engine::runtime::ActionIdent;
use tari_engine_types::instruction::Instruction;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress, ResourceAddress},
};
use tari_template_test_tooling::{support::assert_error::assert_access_denied_for_action, TemplateTest};
use tari_transaction::Transaction;

#[test]
fn basic_faucet_transfer() {
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
            vec![template_test.get_test_proof()],
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

    let (dest_address, non_owning_token, non_owning_key) = template_test.create_owned_account();

    let reason = template_test.execute_expect_failure(
        Transaction::builder()
            .call_method(source_account, "withdraw", args![faucet_resource, Amount(100)])
            .put_last_instruction_output_on_workspace("stolen_coins")
            .call_method(source_account, "deposit", args![Workspace("stolen_coins")])
            .sign(&non_owning_key)
            .build(),
        // VNs provide the token that signed the transaction, which in this case is the non_owning_token
        vec![non_owning_token],
    );

    assert_access_denied_for_action(reason, ActionIdent::ComponentCallMethod {
        method: "withdraw".to_string(),
    });

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

#[test]
fn attempt_to_overwrite_account() {
    let mut template_test = TemplateTest::new::<_, &str>([]);

    // Create sender and receiver accounts
    let (source_account, source_account_proof, source_account_sk) = template_test.create_owned_account();

    template_test.enable_fees();
    let overwriting_tx = template_test.execute_expect_commit(
        Transaction::builder()
            .fee_transaction_pay_from_component(source_account, Amount(1000))
            .call_function(
                template_test.get_template_address("Account"),
                "create",
                args![&source_account_proof],
            )
            // Signed by source account so that it can pay the fees for the new account creation
            .sign(&source_account_sk)
            .build(),
        vec![source_account_proof],
    );

    template_test.disable_fees();

    let result = template_test.execute_expect_success(
        Transaction::builder()
            .call_method(source_account, "get_balances", args![])
            .sign(&source_account_sk)
            .build(),
        vec![],
    );

    let balances = result.finalize.execution_results[0]
        .decode::<Vec<(ResourceAddress, Amount)>>()
        .unwrap();
    // If the source account was overwritten due to the address collision, then we'd have no vaults
    assert_eq!(balances.len(), 1);

    // Now that we know that the component state was not overwritten, lets check that the previous transaction failed
    // because of an address collision.
    overwriting_tx.expect_transaction_failure();
}
