//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
use tari_dan_engine::runtime::{ActionIdent, RuntimeError};
use tari_engine_types::instruction::Instruction;
use tari_template_lib::{
    args,
    auth::AccessRule,
    constants::XTR,
    models::{Amount, ComponentAddress, ResourceAddress},
    prelude::AccessRules,
};
use tari_template_test_tooling::{
    support::assert_error::{assert_access_denied_for_action, assert_reject_reason},
    test_faucet_component,
    TemplateTest,
};
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
    let (sender_address, sender_proof, _) = template_test.create_funded_account();
    let (receiver_address, _, _) = template_test.create_funded_account();

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

    assert_eq!(result.finalize.execution_results[3].decode::<Amount>().unwrap(), 900u64);
    assert_eq!(result.finalize.execution_results[4].decode::<Amount>().unwrap(), 100u64);
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
    let (source_account, _, _) = template_test.create_funded_account();

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

    let (dest_address, non_owning_token, non_owning_key) = template_test.create_funded_account();

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
        component_address: source_account,
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

    // Create initial account with faucet funds
    let (source_account, source_account_proof, source_account_sk) = template_test.create_funded_account();
    let source_account_pk = RistrettoPublicKey::from_secret_key(&source_account_sk);

    let overwriting_tx = template_test.execute_expect_failure(
        Transaction::builder()
            // Create component with the same ID
            .create_account(source_account_pk)
            // Signed by source account so that it can pay the fees for the new account creation
            .sign(&source_account_sk)
            .build(),
        vec![source_account_proof],
    );

    // Check that the previous transaction failed because of an address collision.
    assert_reject_reason(overwriting_tx, RuntimeError::ComponentAlreadyExists {
        address: source_account,
    });

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
    // Double check that the source account was not overwritten due to the address collision, if it was, then we'd have
    // no vaults
    assert_eq!(balances.len(), 1);
}

#[test]
fn gasless() {
    let mut test = TemplateTest::new::<_, &str>([]);
    test.enable_fees();

    // Create initial account with faucet funds
    let (fee_account, fee_account_proof, fee_account_sk) = test.create_funded_account();
    let (user_account, user_account_proof, user_account_sk) = test.create_funded_account();
    let (user2_account, _, _) = test.create_empty_account();

    let result = test.execute_expect_success(
        Transaction::builder()
            .fee_transaction_pay_from_component(fee_account, Amount(1000))
            .call_method(user_account, "withdraw", args![XTR, Amount(100)])
            .put_last_instruction_output_on_workspace("b")
            .call_method(user2_account, "deposit", args![Workspace("b")])
            .call_method(user2_account, "get_balances", args![])
            .build()
            .sign(&fee_account_sk)
            .sign(&user_account_sk),
        vec![fee_account_proof, user_account_proof],
    );

    let balance = result.expect_return::<Vec<(ResourceAddress, Amount)>>(3);
    assert_eq!(balance[0].1, 100);
}

#[test]
fn custom_access_rules() {
    let mut template_test = TemplateTest::new::<_, &str>([]);

    // First we create a account with a custom rule that anyone can withdraw
    let (owner_proof, public_key, secret_key) = template_test.create_owner_proof();

    let access_rules = AccessRules::new()
        .add_method_rule("balance", rule!(allow_all))
        .add_method_rule("get_balances", rule!(allow_all)
        .add_method_rule("deposit", rule!(allow_all)
        .add_method_rule("deposit_all", rule!(allow_all)
        .add_method_rule("get_non_fungible_ids", rule!(allow_all)
        // We are going to make it so anyone can withdraw
        .default(rule!(allow_all));

    let result = template_test.execute_expect_success(
        Transaction::builder()
            .call_method(test_faucet_component(), "take_free_coins", args![])
            .put_last_instruction_output_on_workspace("bucket")
            // Create component with the same ID
            .create_account_with_custom_rules(
                public_key,
                None,
                Some(access_rules),
                Some("bucket"),
            )
            // Signed by source account so that it can pay the fees for the new account creation
            .sign(&secret_key)
            .build(),
        vec![owner_proof],
    );
    let user_account = result.finalize.execution_results[2].decode().unwrap();

    // We create another account and we we will withdraw from the custom one
    let (user2_account, user2_account_proof, user2_secret_key) = template_test.create_funded_account();
    template_test.execute_expect_success(
        Transaction::builder()
            .call_method(user_account, "withdraw", args![XTR, Amount(100)])
            .put_last_instruction_output_on_workspace("b")
            .call_method(user2_account, "deposit", args![Workspace("b")])
            .build()
            .sign(&user2_secret_key),
        vec![user2_account_proof],
    );
}
