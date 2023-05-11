//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::iter;

use tari_engine_types::{commit_result::RejectReason, instruction::Instruction};
use tari_template_lib::{
    args,
    constants::CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
    models::{Amount, ComponentAddress},
};
use tari_template_test_tooling::{test_faucet_component, TemplateTest};
use tari_transaction::Transaction;

#[test]
fn deducts_fees_from_payments_and_refunds_the_rest() {
    let mut test = TemplateTest::new(["tests/templates/state"]);

    let (account, owner_token, private_key) = test.create_owned_account();
    let orig_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);

    test.enable_fees();

    let result = test
        .try_execute_and_commit(
            Transaction::builder()
                .fee_transaction_pay_from_component(account, Amount(1000))
                .call_function(test.get_template_address("State"), "new", args![])
                .sign(&private_key)
                .build(),
            vec![owner_token],
        )
        .unwrap();

    result.expect_success();
    test.disable_fees();

    // Check difference was refunded
    let payment = result.fee_receipt.unwrap();
    let new_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);
    assert_eq!(new_balance, orig_balance - payment.total_fees_charged());
    assert_eq!(payment.total_refunded(), Amount(1000) - payment.total_fees_charged());
    assert!(payment.is_paid_in_full());
}

#[test]
fn deducts_fees_when_transaction_fails() {
    let mut test = TemplateTest::new(["tests/templates/state"]);

    let (account, owner_token, private_key) = test.create_owned_account();
    let orig_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);

    test.enable_fees();

    let result = test
        .try_execute_and_commit(
            Transaction::builder()
                .fee_transaction_pay_from_component(account, Amount(1000))
                .call_function(test.get_template_address("State"), "this_doesnt_exist", args![])
                .sign(&private_key)
                .build(),
            vec![owner_token],
        )
        .unwrap();

    let reason = result.expect_transaction_failure();
    result.expect_finalization_success();
    assert!(matches!(reason, RejectReason::ExecutionFailure(_)));
    test.disable_fees();

    // Check the fee was still paid
    let payment = result.fee_receipt.unwrap();
    let new_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);
    assert_eq!(orig_balance - new_balance, payment.total_fees_charged());
}

#[test]
fn deposit_from_faucet_then_pay() {
    let mut test = TemplateTest::new(["tests/templates/state"]);

    let (account, owner_token, private_key) = test.create_empty_account();

    test.enable_fees();
    let result = test
        .try_execute_and_commit(
            Transaction::builder()
                .with_fee_instructions(vec![
                    Instruction::CallMethod {
                        component_address: test_faucet_component(),
                        method: "take_free_coins".to_string(),
                        args: args![],
                    },
                    Instruction::PutLastInstructionOutputOnWorkspace {
                        key: b"bucket".to_vec(),
                    },
                    Instruction::CallMethod {
                        component_address: account,
                        method: "deposit".to_string(),
                        args: args![Workspace("bucket")],
                    },
                    Instruction::CallMethod {
                        component_address: account,
                        method: "pay_fee".to_string(),
                        args: args![Amount(1000)],
                    },
                ])
                .call_function(test.get_template_address("State"), "new", args![])
                .sign(&private_key)
                .build(),
            vec![owner_token],
        )
        .unwrap();

    result.expect_success();
    test.disable_fees();

    let payment = result.fee_receipt.unwrap();
    let new_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);
    assert_eq!(
        new_balance,
        payment.total_allocated_fee_payments() - payment.total_fees_charged()
    );
}

#[test]
fn another_account_pays_partially_for_fees() {
    let mut test = TemplateTest::new(iter::empty::<&str>());

    let (account, owner_token, private_key) = test.create_empty_account();
    let (account_fee, owner_token_fee, _) = test.create_owned_account();
    let orig_balance: Amount = test.call_method(
        account_fee,
        "balance",
        args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS],
        vec![],
    );

    test.enable_fees();

    let result = test
        .try_execute_and_commit(
            Transaction::builder()
                // Faucet pays a little
                .fee_transaction_pay_from_component(test_faucet_component(), Amount(200))
                // Account pays the rest
                .fee_transaction_pay_from_component(account_fee, Amount(1000))
                .call_method(test_faucet_component(), "take_free_coins", args![])
                .put_last_instruction_output_on_workspace("bucket")
                .call_method(account, "deposit", args![Workspace("bucket")])
                .sign(&private_key)
                .build(),
            vec![owner_token_fee, owner_token],
        )
        .unwrap();

    result.expect_success();
    test.disable_fees();

    // Check difference was refunded
    let payment = result.fee_receipt.unwrap();
    let new_balance: Amount = test.call_method(
        account_fee,
        "balance",
        args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS],
        vec![],
    );
    assert_eq!(new_balance, orig_balance + Amount(200) - payment.total_fees_charged());
    // Check that this test is charging more than just the faucet's portion
    assert!(payment.total_fees_charged() > Amount(200));

    // Check the rest of the transaction was committed
    let balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);
    assert_eq!(balance, Amount(1000));
}

#[test]
fn failed_fee_transaction() {
    let mut test = TemplateTest::new(["tests/templates/state"]);

    let (account, owner_token, private_key) = test.create_owned_account();
    let initial_balance: Amount =
        test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);

    test.enable_fees();
    let result = test
        .try_execute_and_commit(
            Transaction::builder()
                .with_fee_instructions(vec![Instruction::CallMethod {
                    component_address: account,
                    method: "pay_da_fee_plz".to_string(),
                    args: args![],
                }])
                .call_function(test.get_template_address("State"), "new", args![])
                .sign(&private_key)
                .build(),
            vec![owner_token],
        )
        .unwrap();

    let reason = result.expect_failure();
    assert!(matches!(reason, RejectReason::ExecutionFailure(_)));
    let reason = result.expect_transaction_failure();
    assert!(matches!(reason, RejectReason::FeeTransactionFailed));
    test.disable_fees();

    assert!(result.fee_receipt.is_none());
    let new_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);
    assert_eq!(new_balance, initial_balance);
}

#[test]
fn fail_partial_paid_fees() {
    let mut test = TemplateTest::new(["tests/templates/state"]);

    let (account, owner_token, private_key) = test.create_owned_account();
    let (account2, owner_token2, _) = test.create_owned_account();
    let orig_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);

    test.enable_fees();

    let result = test
        .try_execute_and_commit(
            Transaction::builder()
                // Pay less fees than the cost of the main transaction
                .fee_transaction_pay_from_component(account, Amount(10))
                // These instructions should not be applied
                .call_method(account2, "withdraw", args![
                    CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
                    Amount(500)
                ])
                .put_last_instruction_output_on_workspace("bucket")
                .call_method(account, "deposit", args![Workspace("bucket")])
                .sign(&private_key)
                .build(),
            vec![owner_token, owner_token2],
        )
        .unwrap();

    test.disable_fees();

    result.expect_finalization_success();
    let reason = result.expect_transaction_failure();
    assert!(matches!(reason, RejectReason::FeesNotPaid(_)));

    // Check that the fee paid was deducted
    let payment = result.fee_receipt.unwrap();
    assert!(!payment.is_paid_in_full());
    let new_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);
    assert_eq!(new_balance, orig_balance - Amount(10));
}

#[test]
fn fail_pay_less_fees_than_fee_transaction() {
    let mut test = TemplateTest::new(["tests/templates/state"]);

    let (account, owner_token, private_key) = test.create_owned_account();
    let (account2, owner_token2, _) = test.create_owned_account();
    let orig_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);
    let state: ComponentAddress = test.call_function("State", "new", args![], vec![]);

    test.enable_fees();

    let result = test
        .try_execute_and_commit(
            Transaction::builder()
                .with_fee_instructions(
                    // Run up a bill to push it up over the loan
                    (0u32..=10).map(|i| {
                        Instruction::CallMethod {
                            component_address: state,
                            method: "set".to_string(),
                            args: args![i],
                        }
                    })
                    .chain(iter::once(
                        // Pay too little for the fee transaction
                 Instruction::CallMethod {
                            component_address: account,
                            method: "pay_fee".to_string(),
                            args: args![Amount(1)],
                        }
                    ))
                    .collect()
                )
                // These instructions should not be applied
                .call_method(account2, "withdraw", args![
                    CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
                    Amount(500)
                ])
                .put_last_instruction_output_on_workspace("bucket")
                .call_method(account, "deposit", args![Workspace("bucket")])
                .sign(&private_key)
                .build(),
            vec![owner_token, owner_token2],
        )
        .unwrap();

    test.disable_fees();

    let reason = result.expect_failure();
    assert!(matches!(reason, RejectReason::ExecutionFailure(_)));

    // Fee was not deducted
    let new_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);
    assert_eq!(new_balance, orig_balance);

    // State was not updated
    let val: u32 = test.call_method(state, "get", args![], vec![]);
    assert_eq!(val, 0);
}

#[test]
fn fail_pay_too_little_no_fee_instruction() {
    let mut test = TemplateTest::new(iter::empty::<&str>());

    let (account, owner_token, private_key) = test.create_owned_account();
    let (account2, owner_token2, _) = test.create_owned_account();
    let orig_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);

    test.enable_fees();

    let result = test
        .try_execute_and_commit(
            Transaction::builder()
                // These instructions should not be applied
                .call_method(account2, "withdraw", args![
                    CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
                    Amount(500)
                ])
                .put_last_instruction_output_on_workspace("bucket")
                .call_method(account, "deposit", args![Workspace("bucket")])
                .call_method(account,"pay_fee",  args![Amount(10)])
                .sign(&private_key)
                .build(),
            vec![owner_token, owner_token2],
        )
        .unwrap();

    test.disable_fees();

    let reason = result.expect_transaction_failure();
    assert!(matches!(reason, RejectReason::FeesNotPaid(_)));

    // Fee was not deducted
    let new_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);
    assert_eq!(new_balance, orig_balance);
}

#[test]
fn success_pay_fee_in_main_instructions() {
    let mut test = TemplateTest::new(iter::empty::<&str>());

    let (account, owner_token, private_key) = test.create_owned_account();
    let (account2, owner_token2, _) = test.create_owned_account();
    let orig_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);

    test.enable_fees();

    let result = test
        .try_execute_and_commit(
            Transaction::builder()
                .call_method(
                    account2,
                    "withdraw",
                    args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS, Amount(500)],
                )
                .put_last_instruction_output_on_workspace("bucket")
                .call_method(account, "deposit", args![Workspace("bucket")])
                .call_method(account, "pay_fee", args![Amount(1000)])
                .sign(&private_key)
                .build(),
            vec![owner_token, owner_token2],
        )
        .unwrap();

    test.disable_fees();

    result.expect_success();

    let fees = result.expect_fees_paid_in_full();

    // Fee was deducted
    let new_balance: Amount = test.call_method(account, "balance", args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS], vec![]);
    assert_eq!(new_balance, orig_balance + Amount(500) - fees.total_fees_charged());
}
