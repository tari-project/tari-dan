//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::vec;

use tari_crypto::ristretto::RistrettoSecretKey;
use tari_dan_engine::runtime::{AssertError, RuntimeError};
use tari_template_lib::{args, models::{Amount, ComponentAddress, NonFungibleAddress, ResourceAddress}, prelude::XTR};
use tari_template_test_tooling::{support::assert_error::assert_reject_reason, TemplateTest};
use tari_transaction::{Instruction, Transaction};

const FAUCET_WITHDRAWAL_AMOUNT: Amount = Amount::new(1000);

struct AssertTest {
    template_test: TemplateTest,
    faucet_component: ComponentAddress,
    faucet_resource: ResourceAddress,
    account: ComponentAddress,
    account_proof: NonFungibleAddress,
    account_key: RistrettoSecretKey,
}

fn setup() -> AssertTest {
    let mut template_test = TemplateTest::new(vec!["tests/templates/tariswap"]);

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
    let (account, account_proof, account_key) = template_test.create_funded_account();

    AssertTest {
        template_test,
        faucet_component,
        faucet_resource,
        account,
        account_proof,
        account_key
    }
}

#[test]
fn successful_assert() {
    let mut test: AssertTest = setup();

    test.template_test.execute_expect_success(
        Transaction::builder()
            .call_method(test.faucet_component, "take_free_coins", args![])
            .put_last_instruction_output_on_workspace("faucet_bucket")
            .assert_bucket_contains("faucet_bucket", test.faucet_resource, FAUCET_WITHDRAWAL_AMOUNT)
            .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
            .sign(&test.account_key)
            .build(),
        vec![test.account_proof.clone()],
    );
}

#[test]
fn it_fails_with_invalid_resource() {
    let mut test: AssertTest = setup();

    // we are going to assert a different resource than the faucet resource
    let invalid_resource_address = XTR;

    let reason = test.template_test.execute_expect_failure(
        Transaction::builder()
            .call_method(test.faucet_component, "take_free_coins", args![])
            .put_last_instruction_output_on_workspace("faucet_bucket")
            .assert_bucket_contains("faucet_bucket", invalid_resource_address, FAUCET_WITHDRAWAL_AMOUNT)
            .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
            .sign(&test.account_key)
            .build(),
        vec![test.account_proof.clone()],
    );

    assert_reject_reason(reason, RuntimeError::AssertError(AssertError::InvalidResource { expected: invalid_resource_address, got: test.faucet_resource }));
}

#[test]
fn it_fails_with_invalid_amount() {
    let mut test: AssertTest = setup();

    // we are going to assert that the faucet bucket has more tokens that it really has
    let min_amount = FAUCET_WITHDRAWAL_AMOUNT + 1;

    let reason = test.template_test.execute_expect_failure(
        Transaction::builder()
            .call_method(test.faucet_component, "take_free_coins", args![])
            .put_last_instruction_output_on_workspace("faucet_bucket")
            .assert_bucket_contains("faucet_bucket", test.faucet_resource, min_amount)
            .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
            .sign(&test.account_key)
            .build(),
        vec![test.account_proof.clone()],
    );

    assert_reject_reason(reason, RuntimeError::AssertError(AssertError::InvalidAmount { expected: min_amount, got: FAUCET_WITHDRAWAL_AMOUNT}));
}

#[test]
fn it_fails_with_invalid_bucket() {
    let mut test: AssertTest = setup();

    let reason = test.template_test.execute_expect_failure(
        Transaction::builder()
            .call_method(test.faucet_component, "take_free_coins", args![])
            // we are going to assert a workspace value that is NOT a bucket
            .call_method(test.account, "get_balances", args![])
            .put_last_instruction_output_on_workspace("invalid_bucket")
            .assert_bucket_contains("invalid_bucket", test.faucet_resource, FAUCET_WITHDRAWAL_AMOUNT)
            .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
            .sign(&test.account_key)
            .build(),
        vec![test.account_proof.clone()],
    );

    assert_reject_reason(reason, RuntimeError::AssertError(AssertError::InvalidBucket {}));
}

#[test]
fn it_fails_with_invalid_workspace_key() {
    let mut test: AssertTest = setup();

    let reason = test.template_test.execute_expect_failure(
        Transaction::builder()
            .call_method(test.faucet_component, "take_free_coins", args![])
            .put_last_instruction_output_on_workspace("faucet_bucket")
            // we are going to assert a key that does not exist in the workspace
            .assert_bucket_contains("invalid_key", test.faucet_resource, FAUCET_WITHDRAWAL_AMOUNT)
            .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
            .sign(&test.account_key)
            .build(),
        vec![test.account_proof.clone()],
    );

    assert_reject_reason(reason, RuntimeError::ItemNotOnWorkspace { key: "invalid_key".to_string() });
}