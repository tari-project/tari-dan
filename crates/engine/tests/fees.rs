//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine::fees::FeeTable;
use tari_engine_types::{
    commit_result::{RejectReason, TransactionResult},
    fees::{FeeReceipt, FeeSource},
    limits::ENGINE_LIMITS,
};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::types::{
    Amount,
    ComponentAddress,
    NonFungibleAddress,
    constants::STEALTH_TARI_RESOURCE_ADDRESS,
};
use tari_template_test_tooling::{
    TemplateTest,
    compile::compile_template,
    support::assert_error::assert_reject_reason,
    xtr_faucet_component,
};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");
const TEMPLATE_PATHS: [&str; 1] = ["tests/templates/state"];

#[test]
fn deducts_fees_from_payments_and_refunds_the_rest() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);

    let (account, owner_token, private_key) = test.create_funded_account();
    let orig_balance: Amount = test.call_method(account, "balance", args![STEALTH_TARI_RESOURCE_ADDRESS], vec![]);

    test.enable_fees();

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account, 1000u64)
            .call_function(test.get_template_address("State"), "new", args![])
            .build_and_seal(&private_key),
        vec![owner_token],
    );

    test.disable_fees();

    // Check difference was refunded
    let payment = result.finalize.fee_receipt.clone();
    let new_balance = test
        .read_only_state_store()
        .get_vaults_for_account(account)
        .unwrap()
        .get(&STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap()
        .balance();
    assert_eq!(new_balance, orig_balance - payment.total_fees_charged());
    assert_eq!(payment.total_refunded(), 1000 - payment.total_fees_charged());
    assert!(payment.is_paid_in_full());
}

/// A log's message is charged for by the byte, so filling the per-transaction log budget costs in
/// proportion to what it carries rather than a flat per-call fee.
#[test]
fn a_log_is_charged_for_the_bytes_it_carries() {
    fn runtime_call_charge(message_len: usize) -> (u64, FeeTable) {
        let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/limits"]);
        let (account, owner_token, private_key) = test.create_funded_account();
        test.enable_fees();

        let result = test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .pay_fee_from_component(account, 100_000u64)
                .call_function(
                    test.get_template_address("PushItToTheLimit"),
                    "emit_log_of_size",
                    args![message_len as u32],
                )
                .build_and_seal(&private_key),
            vec![owner_token],
        );
        (
            result.finalize.fee_receipt.fee_breakdown().get(FeeSource::RuntimeCall),
            test.fee_table().clone(),
        )
    }

    let bytes = ENGINE_LIMITS.max_log_size_bytes;
    let (with_log, table) = runtime_call_charge(bytes);
    let (without_log, _) = runtime_call_charge(0);

    // Both runs make the same host calls, so the difference is what the message carried.
    let charged = with_log - without_log;
    assert_eq!(
        charged,
        table.per_byte_storage_cost() * bytes as u64 / table.log_bytes_cost_divisor()
    );
    assert!(charged > 0, "a max-size log message was charged nothing");

    // The division is per call, so a message shorter than the divisor rounds to nothing. What that
    // forgoes over a transaction is bounded by `max_logs × log_bytes_cost_divisor` bytes.
    let short = table.log_bytes_cost_divisor() as usize - 1;
    let (with_short_log, _) = runtime_call_charge(short);
    assert_eq!(with_short_log, without_log);
}

#[test]
fn deducts_fees_when_transaction_fails() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);

    let (account, owner_token, private_key) = test.create_funded_account();
    let vaults = test.read_only_state_store().get_vaults_for_account(account).unwrap();
    let orig_balance = vaults.get(&STEALTH_TARI_RESOURCE_ADDRESS).unwrap().balance();

    test.enable_fees();

    let result = test.execute_and_commit_on_success(
        Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account, 1000u64)
            .call_function(test.get_template_address("State"), "this_doesnt_exist", args![])
            .build_and_seal(&private_key),
        vec![owner_token],
    );

    let reason = result.expect_failure();
    result.expect_finalization_success();
    assert!(matches!(reason, RejectReason::ExecutionFailure(_)));

    // Check the fee was still paid
    let payment = result.finalize.fee_receipt;
    let vaults = test.read_only_state_store().get_vaults_for_account(account).unwrap();
    let new_balance = vaults.get(&STEALTH_TARI_RESOURCE_ADDRESS).unwrap().balance();
    assert!(payment.total_fees_paid() > 0);
    assert_eq!(orig_balance - new_balance, payment.total_fees_paid());
}

#[test]
fn deposit_from_faucet_then_pay() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);

    let (account, owner_token, private_key) = test.create_empty_account();

    test.enable_fees();

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| {
                builder
                    // Faucet deposits free coins into the account
                    .call_method(xtr_faucet_component(), "take", args![account])
                    .call_method(account, "pay_fee", args![3000])
            })
            .call_function(test.get_template_address("State"), "new", args![])
            .build_and_seal(&private_key),
        vec![owner_token],
    );

    let payment = result.finalize.fee_receipt;
    let new_balance = test
        .read_only_state_store()
        .get_vaults_for_account(account)
        .unwrap()
        .get(&STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap()
        .balance();
    assert_eq!(new_balance, 1_000_000_000 - payment.total_fees_paid());
}

#[test]
fn another_account_pays_partially_for_fees() {
    let mut test = TemplateTest::new_builtin_only();

    let (account, _, _) = test.create_empty_account();
    let (account_fee, owner_token_fee, _) = test.create_funded_account();
    let (account_fee2, owner_token_fee2, _) = test.create_funded_account();
    let orig_balance: Amount = test.call_method(account_fee, "balance", args![STEALTH_TARI_RESOURCE_ADDRESS], vec![]);

    test.enable_fees();

    // Faucet's cap must be smaller than the total fee for this transaction so that
    // account_fee2 is forced to cover the remainder (the point of this test).
    const FAUCET_CAP: u64 = 100;

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            // Faucet pays a little
            .pay_fee_from_component(account_fee, Amount::from(FAUCET_CAP))
            // Account pays the rest
            .pay_fee_from_component(account_fee2, Amount::from(3000u64))
            .call_method(xtr_faucet_component(), "take", args![account])
            // NOTE: the test harness provides the virtual proofs as provided, so the transaction signer does not matter
            .build_and_seal(test.secret_key()),
        vec![owner_token_fee, owner_token_fee2],
    );

    test.disable_fees();

    // Check difference was refunded
    let payment = result.finalize.fee_receipt;
    let vaults = test
        .read_only_state_store()
        .get_vaults_for_account(account_fee2)
        .unwrap();
    let vault = vaults.get(&STEALTH_TARI_RESOURCE_ADDRESS).unwrap();

    assert_eq!(
        vault.balance(),
        orig_balance + Amount::from(FAUCET_CAP) - Amount::from(payment.total_fees_charged())
    );
    // Check that this test is charging more than just the faucet's portion
    assert!(Amount::from(FAUCET_CAP) < payment.total_fees_charged());

    // Check the rest of the transaction was committed
    let balance = test
        .read_only_state_store()
        .get_vaults_for_account(account)
        .unwrap()
        .get(&STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap()
        .balance();
    assert_eq!(balance, Amount::from(1_000_000_000u64));
}

#[test]
fn failed_fee_transaction() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);

    let (account, owner_token, private_key) = test.create_funded_account();
    let initial_balance: Amount = test.call_method(account, "balance", args![STEALTH_TARI_RESOURCE_ADDRESS], vec![]);

    test.enable_fees();
    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .with_fee_instructions_builder(|builder| {
                    builder
                        // This instruction will fail
                        .call_method(account, "pay_da_fee_plz", args![])
                })
                .call_function(test.get_template_address("State"), "new", args![])
                .build_and_seal(&private_key),
            vec![owner_token],
        )
        .unwrap();

    let reason = result.expect_finalization_failure();
    assert!(matches!(reason, RejectReason::ExecutionFailure(_)));
    let reason = result.expect_failure();
    assert!(matches!(reason, RejectReason::ExecutionFailure(_)));

    let new_balance = test
        .read_only_state_store()
        .get_vaults_for_account(account)
        .unwrap()
        .get(&STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap()
        .balance();
    assert_eq!(new_balance, initial_balance);
}

/// Runs a transaction that creates several components, paying `fee` towards it, and returns the
/// storage fee it was charged along with whether the main intent committed.
fn storage_charged_for_creating_components(fee: u64) -> (u64, bool) {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (account, owner_token, private_key) = test.create_funded_account();
    test.enable_fees();

    let state_template = test.get_template_address("State");
    let mut builder = test.transaction().pay_fee_from_component(account, Amount::from(fee));
    for _ in 0..5 {
        builder = builder.call_function(state_template, "new", args![]);
    }

    let result = test.execute_expect_commit(builder.build_and_seal(&private_key), vec![owner_token]);
    let committed = result.finalize.result.is_accept();
    (
        result.finalize.fee_receipt.fee_breakdown().get(FeeSource::Storage),
        committed,
    )
}

/// A transaction that cannot pay for its main intent commits only its fee intent, so the storage it
/// is charged for is the fee checkpoint's — not that of the state it built and abandoned.
#[test]
fn a_fee_intent_commit_is_not_charged_for_the_state_it_abandons() {
    let (storage_rejected, committed) = storage_charged_for_creating_components(1_000);
    assert!(!committed, "expected the main intent to be rejected");

    let (storage_committed, committed) = storage_charged_for_creating_components(100_000);
    assert!(committed, "expected the main intent to commit");

    // The components only exist in the committed run, so only it pays for them. Charging the
    // rejected run over the working state instead of the checkpoint would put the two within a few
    // bytes of each other.
    assert!(
        storage_rejected * 2 < storage_committed,
        "rejected run charged {storage_rejected} storage against the committed run's {storage_committed}"
    );
}

/// Compute is funded by what the payment has left after the state the transaction falls back to
/// committing, not by the whole payment.
///
/// Funding it from the whole payment let a transaction spend the lot on execution and leave its own
/// fee-intent commit unaffordable. That settles as a rejection which collects nothing, so the work
/// was done and never paid for — repeatably, with the same funds each time.
#[test]
fn compute_cannot_consume_what_the_fee_intent_commit_needs() {
    for burn_rate_bps in [0, 500, 10_000] {
        spend_the_whole_allowance_and_still_pay(burn_rate_bps);
    }
}

fn spend_the_whole_allowance_and_still_pay(burn_rate_bps: u16) {
    const MAX_FEE: u64 = 100_000;

    let mut test = TemplateTest::new(CRATE_PATH, vec!["tests/templates/infinity_loop"]);
    let (account, owner_token, key) = test.create_funded_account();
    test.enable_fees();

    // Run at a burn too: the burn is a share of what is paid, so it must not narrow what the
    // payment can fund nor leave any of it uncollected.
    test.set_burn_rate_bps(burn_rate_bps);

    // A loop that never returns: it runs until the compute allowance stops it, whatever that
    // allowance is, so the transaction always spends every point it is given.
    let template = test.get_template_address("InfinityLoopTest");
    let result = test.execute_expect_commit(
        test.transaction()
            .pay_fee_from_component(account, MAX_FEE)
            .call_function(template, "infinity_loop", args![])
            .build_and_seal(&key),
        vec![owner_token],
    );

    // The main intent traps, so only the fee intent commits — and what is left of the payment still
    // covers it, so the fees are collected.
    assert!(
        matches!(result.finalize.result, TransactionResult::AcceptFeeRejectRest(..)),
        "expected a fee-intent commit at {burn_rate_bps} bps, got {:?}",
        result.finalize.result
    );
    let receipt = &result.finalize.fee_receipt;
    assert!(receipt.is_paid_in_full(), "at {burn_rate_bps} bps");
    assert_eq!(
        receipt.total_fees_charged(),
        receipt.total_fees_paid(),
        "at {burn_rate_bps} bps the transaction should spend the payment down to the microtari"
    );
    // The allowance is sized against the payment and not beyond it, so the loop spends the payment
    // to the last microtari the metering divisor can resolve.
    assert!(
        receipt.total_fees_paid() > MAX_FEE - 100,
        "at {burn_rate_bps} bps only {} of {MAX_FEE} was spent",
        receipt.total_fees_paid()
    );
    // The burn is taken out of what was paid, not added to it.
    let expected_burn = u128::from(receipt.total_fees_paid()) * u128::from(burn_rate_bps) / 10_000;
    assert_eq!(
        u128::from(receipt.exhaust_burn()),
        expected_burn,
        "at {burn_rate_bps} bps"
    );
    assert_eq!(
        receipt.pre_burn_fees_paid() + receipt.exhaust_burn(),
        receipt.total_fees_paid(),
        "at {burn_rate_bps} bps"
    );
}

/// A fee-intent commit persists real state, so it happens only when the payment covers what that
/// state costs. When it does not, nothing commits — there is no shallower checkpoint to fall back
/// to, and committing anyway would be a way to write state for free.
#[test]
fn an_unaffordable_fee_intent_commits_nothing() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (account, owner_token, private_key) = test.create_empty_account();
    test.enable_fees();

    // Enough for the fee intent's execution, nowhere near the state it writes.
    const FEE: u64 = 100;

    let result = test
        .try_execute(
            test.transaction()
                .with_fee_instructions_builder(|builder| {
                    builder
                        // Writes the account's vault and the faucet's, then pays a fraction of what
                        // storing them costs.
                        .call_method(xtr_faucet_component(), "take", args![account])
                        .call_method(account, "pay_fee", args![FEE])
                })
                .build_and_seal(&private_key),
            vec![owner_token],
        )
        .unwrap();

    assert!(
        matches!(
            result.finalize.result,
            TransactionResult::Reject(RejectReason::InsufficientFeesPaid(_))
        ),
        "actual result: {:?}",
        result.finalize.result
    );

    // Nothing was written, so nothing was taken.
    assert_eq!(result.finalize.fee_receipt.total_fees_paid(), 0);

    // What committing would have cost is still reported: it is the number the payer has to raise
    // their fee to, and on a rejection there is nowhere else to read it from.
    let charged = result.finalize.fee_receipt.total_fees_charged();
    assert!(charged > FEE, "expected the metered charges, got {charged}");
    assert!(result.finalize.required_fees() > FEE);

    let balance = test
        .read_only_state_store()
        .get_vaults_for_account(account)
        .unwrap()
        .get(&STEALTH_TARI_RESOURCE_ADDRESS)
        .map(|v| v.balance());
    assert!(
        balance.is_none() || balance == Some(Amount::zero()),
        "account was funded: {balance:?}"
    );
}

#[test]
fn fail_partial_paid_fees() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);

    let (account, owner_token, private_key) = test.create_funded_account();
    let (account2, owner_token2, _) = test.create_funded_account();
    let orig_balance: Amount = test.call_method(account, "balance", args![STEALTH_TARI_RESOURCE_ADDRESS], vec![]);
    test.enable_fees();

    // Must cover what committing the fee intent costs — otherwise nothing commits at all — yet stay
    // smaller than the full transaction's fee, so the main instructions exhaust the compute the
    // payment funds and trap.
    const FEE_PAID: u64 = 1000;

    let result = test.execute_expect_commit(
        Transaction::builder_localnet(Epoch(1))
            // Pay less fees than the cost of the main transaction
            .pay_fee_from_component(account, Amount::from(FEE_PAID))
            // These instructions should not be applied
            .call_method(account2, "withdraw", args![
                    STEALTH_TARI_RESOURCE_ADDRESS,
                    1000
                ])
            .put_last_instruction_output_on_workspace("bucket")
            .take_from_bucket("bucket", 500u64, "bucket2")
            .call_method(account, "deposit", args![Workspace("bucket")])
            .call_method(account, "deposit", args![Workspace("bucket2")])
            .build_and_seal(&private_key),
        vec![owner_token, owner_token2],
    );

    // The main instructions cost more than was paid, so they are rejected and only the fee intent
    // commits.
    let reason = result.expect_failure();
    assert!(
        matches!(reason, RejectReason::InsufficientFeesPaid(_)),
        "actual reason: {reason}"
    );

    // What is charged is what the fee intent persisted, not what the abandoned instructions would
    // have. That is below the payment, so the commit is affordable and the rest is refunded.
    let payment = &result.finalize.fee_receipt;
    let total_fees = payment.fee_breakdown().get_total();
    assert!(
        total_fees < FEE_PAID,
        "total fees {total_fees} exceeds the {FEE_PAID} paid"
    );
    assert!(payment.is_paid_in_full());
    assert_eq!(payment.total_refunded(), FEE_PAID - total_fees);

    // The fee intent's own cost is below what was paid, so it cannot tell a resubmission what to
    // clear. The result keeps the figure the main intent was rejected for.
    assert!(
        result.finalize.total_fees_required > FEE_PAID,
        "the transaction was rejected for underpayment, so what it required must exceed the {FEE_PAID} paid"
    );
    assert!(result.finalize.required_fees() > result.finalize.total_fees_required);

    let new_balance = test
        .read_only_state_store()
        .get_vaults_for_account(account)
        .unwrap()
        .get(&STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap()
        .balance();
    assert_eq!(new_balance, orig_balance - Amount::from(total_fees));
}

#[test]
fn fail_pay_negative_fee() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);

    let (account, owner_token, private_key) = test.create_funded_account();
    test.enable_fees();

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| builder.call_method(account, "pay_fee", args![-100]))
            .build_and_seal(&private_key),
        vec![owner_token],
    );

    assert!(
        matches!(reason, RejectReason::ExecutionFailure(_)),
        "actual reason: {reason}"
    );
}

#[test]
fn fail_pay_less_fees_than_fee_transaction() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);

    let (account, owner_token, private_key) = test.create_funded_account();
    let (account2, owner_token2, _) = test.create_funded_account();
    let orig_balance: Amount = test.call_method(account, "balance", args![STEALTH_TARI_RESOURCE_ADDRESS], vec![]);
    let state: ComponentAddress = test.call_function("State", "new", args![], vec![]);

    test.enable_fees();

    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .with_fee_instructions_builder(|builder| {
                    (0u32..=0).fold(builder, |builder, i| {
                        builder.call_method(
                            state,
                            "set".to_string(),
                            args![i],
                        )
                    })
                        .call_method(
                            account,
                            "pay_fee".to_string(),
                            // Less than the fee instructions themselves cost to commit, so not
                            // even the fee intent survives.
                            args![150],
                        )

                })
                // These instructions should not be applied
                .call_method(account2, "withdraw", args![
                    STEALTH_TARI_RESOURCE_ADDRESS,
                    500
                ])
                .put_last_instruction_output_on_workspace("bucket")
                .call_method(account, "deposit", args![Workspace("bucket")])
                .build_and_seal(&private_key),
            vec![owner_token, owner_token2],
        )
        .unwrap();

    test.disable_fees();

    // The payment does not cover the state the fee intent writes, so nothing commits at all.
    assert!(
        matches!(result.finalize.result, TransactionResult::Reject(_)),
        "actual result: {:?}",
        result.finalize.result
    );
    assert_reject_reason(
        result.expect_failure(),
        RejectReason::InsufficientFeesPaid(String::new()),
    );

    // Fee was not deducted
    let new_balance = test
        .read_only_state_store()
        .get_vaults_for_account(account)
        .unwrap()
        .get(&STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap()
        .balance();
    assert_eq!(new_balance, orig_balance);

    // State was not updated. minicbor encodes enums as `[variant_index, [fields...]]`,
    // so State::Zero — a unit variant — is `[0, []]`.
    let component_state = test.read_only_state_store().get_component(state).unwrap();
    let arr = component_state
        .body
        .state
        .as_array()
        .expect("State should encode as a CBOR array");
    assert_eq!(arr.len(), 2, "expected [variant_index, [fields]]");
    assert_eq!(arr[0].as_integer(), Some(0), "expected the Zero variant tag");
    assert_eq!(
        arr[1].as_array().map(<[_]>::len),
        Some(0),
        "expected the unit variant to have an empty fields array"
    );
}

#[test]
fn fail_pay_too_little_no_fee_instruction() {
    let mut test = TemplateTest::new_builtin_only();

    let (account, owner_token, private_key) = test.create_funded_account();
    let (account2, owner_token2, _) = test.create_funded_account();
    let orig_balance: Amount = test.call_method(account, "balance", args![STEALTH_TARI_RESOURCE_ADDRESS], vec![]);

    test.enable_fees();

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| {
                builder
                    // These instructions should not be applied
                    .call_method(account2, "withdraw", args![
                        STEALTH_TARI_RESOURCE_ADDRESS,
                        500
                    ])
                    .put_last_instruction_output_on_workspace("bucket")
                    .call_method(account, "deposit", args![Workspace("bucket")])
                    .call_method(account, "pay_fee", args![10])
            })
            .build_and_seal(&private_key),
        vec![owner_token, owner_token2],
    );

    assert!(
        matches!(reason, RejectReason::InsufficientFeesPaid(_)),
        "actual reason: {reason}"
    );

    // Fee was not deducted
    let new_balance = test
        .read_only_state_store()
        .get_vaults_for_account(account)
        .unwrap()
        .get(&STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap()
        .balance();
    assert_eq!(new_balance, orig_balance);
}

#[test]
fn failure_pay_fee_in_main_instructions() {
    let mut test = TemplateTest::new_builtin_only();

    let (account, owner_token, private_key) = test.create_funded_account();

    test.enable_fees();

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            // Pay in the fee intent enough to cover committing it, so the transaction reaches the
            // main instructions where the violation is.
            .pay_fee_from_component(account, 2000u64)
            // Call pay_fee in main instructions (outside fee instructions) not permitted
            .call_method(account, "pay_fee", args![100])
            .call_method(account, "balance", args![STEALTH_TARI_RESOURCE_ADDRESS])
            .call_method(account, "balance", args![STEALTH_TARI_RESOURCE_ADDRESS])
            .build_and_seal(&private_key),
        vec![owner_token],
    );

    assert_reject_reason(reason, RejectReason::FeePaymentInMainIntent);
}

#[test]
fn dangling_bucket_pay_fees() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);

    let (account, owner_token, private_key) = test.create_funded_account();
    let orig_balance: Amount = test.call_method(account, "balance", args![STEALTH_TARI_RESOURCE_ADDRESS], vec![]);

    test.enable_fees();

    let result = test.execute_and_commit_on_success(
        test.transaction()
            .pay_fee_from_component(account, Amount::from(3000u64))
            .call_method(account, "withdraw", args![STEALTH_TARI_RESOURCE_ADDRESS, 10])
            .put_last_instruction_output_on_workspace("dangling_bucket")
            .build_and_seal(&private_key),
        vec![owner_token],
    );

    // Check that the failure reason is actually the dangling bucket
    let reason = result.expect_failure();
    assert!(matches!(reason, RejectReason::ExecutionFailure(_)));
    assert!(reason.to_string().contains("dangling bucket"));

    // The transaction still finishes successfully
    result.expect_finalization_success();

    // Check the fee was still paid
    let payment = result.finalize.fee_receipt;
    let new_balance = test
        .read_only_state_store()
        .get_vaults_for_account(account)
        .unwrap()
        .get(&STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap()
        .balance();
    assert_ne!(payment.total_fees_paid(), 0);
    assert_eq!(orig_balance - new_balance, payment.total_fees_paid());
}

// A submitted `max_fee` is itself an input to what the transaction costs, so a dry run metered at
// one `max_fee` does not perfectly predict a real run at another. There are three quantities the
// cost could read `max_fee` through, and what a caller can rely on is which of them it does:
//
// - the encoded *width* of the fee literal, through `calc_args_weight`. Live, and the only term that can make a real
//   run cost more than the dry run said.
// - the fee amount's *digit count*, through the `std.vault.pay_fee` event the persisted receipt carries. Neutralized —
//   priced at its widest, so it buys nothing.
// - the *residual vault balance*'s width, since the finalization charges run before refunds. Live, and opposed to the
//   first: a wider `max_fee` leaves a narrower residual. Nothing narrows the `max_fee` a dry run meters at, so a
//   submission can move either term in either direction and the allowance has to cover both.
//
// `try_execute` never commits, so every run below starts from identical state and the whole delta
// is attributable to `max_fee`. Each test varies one of the three quantities and holds the other
// two fixed, so a failure names its mechanism rather than a total.

/// The balance `create_funded_account` starts an account with, so the residual vault balance the
/// byte counter sees (`FUNDED - max_fee`) is predictable.
const FUNDED: u64 = 1_000_000_000;

/// `calc_args_weight` prices an instruction's literal args by their encoded bytes, so the weight
/// charge reads `max_fee` through the width of its encoding.
fn amount_len(value: u64) -> u64 {
    tari_bor::encode(&Amount::from(value)).unwrap().len() as u64
}

/// The `std.vault.pay_fee` event records the amount as a decimal string, so the receipt's size — and
/// with it the storage charge — would read `max_fee` through its digit count if the amount were not
/// priced at its widest.
fn digit_count(value: u64) -> u64 {
    value.to_string().len() as u64
}

/// Meters `build(max_fee)` once per `max_fee`, from identical state each time.
fn meter_across_max_fees(
    test: &mut TemplateTest,
    max_fees: &[u64],
    proofs: &[NonFungibleAddress],
    build: impl Fn(u64) -> Transaction,
) -> Vec<FeeReceipt> {
    max_fees
        .iter()
        .map(|&max_fee| {
            let result = test.try_execute(build(max_fee), proofs.to_vec()).unwrap();
            // A run that does not commit meters a different execution — only the fee intent — and
            // would not be comparable with the others.
            assert!(
                matches!(result.finalize.result, TransactionResult::Accept(_)),
                "max_fee {max_fee} did not commit, so this run is not comparable: {:?}",
                result.finalize.result
            );
            result.finalize.fee_receipt
        })
        .collect()
}

fn state_transaction<'a>(
    test: &TemplateTest,
    account: ComponentAddress,
    key: &'a RistrettoSecretKey,
) -> impl Fn(u64) -> Transaction + use<'a> {
    let template = test.get_template_address("State");
    let tx = test.transaction();
    move |max_fee| {
        tx.clone()
            .pay_fee_from_component(account, max_fee)
            .call_function(template, "new", args![])
            .build_and_seal(key)
    }
}

/// `max_fee` is the only literal arg of the `pay_fee` instruction, and `calc_args_weight` prices an
/// instruction's literals at `total_bytes / LITERAL_BYTE_DIVISOR`. The whole weight of this
/// transaction is that one term, so it steps whenever the encoded width crosses the divisor.
#[test]
fn transaction_weight_follows_the_max_fee_literal_width() {
    const LITERAL_BYTE_DIVISOR: u64 = 3;
    // Straddles an encoding-width boundary while keeping the digit count and the residual balance's
    // width fixed, so the weight charge is the only thing that can move.
    const MAX_FEES: [u64; 2] = [65_535, 65_536];

    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (account, owner_token, key) = test.create_funded_account();
    test.enable_fees();
    let build = state_transaction(&test, account, &key);
    let receipts = meter_across_max_fees(&mut test, &MAX_FEES, &[owner_token], build);

    assert_ne!(amount_len(MAX_FEES[0]), amount_len(MAX_FEES[1]));
    assert_eq!(digit_count(MAX_FEES[0]), digit_count(MAX_FEES[1]));

    let per_weight = test.fee_table().per_transaction_weight_cost();
    for (max_fee, receipt) in MAX_FEES.iter().zip(&receipts) {
        assert_eq!(
            receipt.fee_breakdown().get(FeeSource::TransactionWeight),
            (amount_len(*max_fee) / LITERAL_BYTE_DIVISOR) * per_weight,
            "TransactionWeight at max_fee {max_fee}"
        );
        assert_eq!(
            receipt.fee_breakdown().get(FeeSource::Storage),
            receipts[0].fee_breakdown().get(FeeSource::Storage),
            "Storage must not move while the digit count and residual width hold"
        );
    }
}

/// The transaction receipt is part of the state a transaction pays to persist, and it carries the
/// `std.vault.pay_fee` event, whose payload records the amount as a decimal string. Charging that
/// verbatim would price permanent state by the digit count of `max_fee`, so the amount is priced at
/// its widest instead and the digit count buys nothing.
#[test]
fn storage_does_not_follow_the_max_fee_digit_count() {
    // One encoding width and one residual width throughout, so the digit count is the only thing
    // that varies.
    const MAX_FEES: [u64; 4] = [65_536, 1_000_000, 100_000_000, 999_000_000];

    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (account, owner_token, key) = test.create_funded_account();
    test.enable_fees();
    let build = state_transaction(&test, account, &key);
    let receipts = meter_across_max_fees(&mut test, &MAX_FEES, &[owner_token], build);

    assert_ne!(
        digit_count(MAX_FEES[0]),
        digit_count(MAX_FEES[MAX_FEES.len() - 1]),
        "the digit count must actually vary for this to prove anything"
    );
    for (max_fee, receipt) in MAX_FEES.iter().zip(&receipts) {
        assert_eq!(
            amount_len(*max_fee),
            amount_len(MAX_FEES[0]),
            "encoding width must hold"
        );
        assert_eq!(
            amount_len(FUNDED - max_fee),
            amount_len(FUNDED - MAX_FEES[0]),
            "residual width must hold"
        );
        assert_eq!(
            receipt.fee_breakdown().get(FeeSource::Storage),
            receipts[0].fee_breakdown().get(FeeSource::Storage),
            "Storage at max_fee {max_fee}"
        );
    }
}

/// The finalization charges run before `finalize_fees_and_refunds` returns the unspent payment, so
/// the fee vault is byte-counted holding `balance - max_fee`. A larger `max_fee` narrows that
/// residual and makes storage *cheaper* — the opposite direction to the digit-count term above.
#[test]
fn storage_follows_the_residual_vault_balance_width() {
    // One encoding width and one digit count throughout, so the residual width is the only thing
    // that varies.
    const MAX_FEES: [u64; 3] = [999_940_000, 999_999_800, 999_999_990];

    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (account, owner_token, key) = test.create_funded_account();
    test.enable_fees();
    let build = state_transaction(&test, account, &key);
    let receipts = meter_across_max_fees(&mut test, &MAX_FEES, &[owner_token], build);

    let per_byte = test.fee_table().per_byte_storage_cost();
    let divisor = test.fee_table().storage_cost_divisor();
    for (max_fee, receipt) in MAX_FEES.iter().zip(&receipts) {
        assert_eq!(
            amount_len(*max_fee),
            amount_len(MAX_FEES[0]),
            "encoding width must hold"
        );
        assert_eq!(digit_count(*max_fee), digit_count(MAX_FEES[0]), "digit count must hold");
        let bytes_saved = amount_len(FUNDED - MAX_FEES[0]) - amount_len(FUNDED - max_fee);
        assert_eq!(
            receipts[0].fee_breakdown().get(FeeSource::Storage) - receipt.fee_breakdown().get(FeeSource::Storage),
            bytes_saved * per_byte / divisor,
            "Storage at max_fee {max_fee}"
        );
    }
}

/// Only `TransactionWeight` and `Storage` are `max_fee`-sensitive. Anything else moving would mean
/// the three mechanisms above do not account for the whole drift.
#[test]
fn no_charge_other_than_weight_and_storage_moves_with_max_fee() {
    const MAX_FEES: [u64; 6] = [1_000, 65_535, 65_536, 100_000_000, FUNDED - 60_000, FUNDED - 10];

    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (account, owner_token, key) = test.create_funded_account();
    test.enable_fees();
    let build = state_transaction(&test, account, &key);
    let receipts = meter_across_max_fees(&mut test, &MAX_FEES, &[owner_token], build);

    assert_only_weight_and_storage_move(&MAX_FEES, &receipts);
}

/// A publish carries a blob and takes the dedicated `TemplatePublish` pricing path, so it is the
/// shape most likely to hide a fourth mechanism. It does not.
#[test]
fn a_template_publish_introduces_no_further_max_fee_sensitivity() {
    // Every entry clears the publish cost; between them they move all three quantities.
    const MAX_FEES: [u64; 4] = [400_000, 100_000_000, FUNDED - 60_000, FUNDED - 10];

    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account, owner_proof, key, _) = test.create_funded_account_with_keypair();
    let template = compile_template("tests/templates/hello_world", &[]).unwrap();
    test.enable_fees();

    let tx = test.transaction();
    let receipts = meter_across_max_fees(&mut test, &MAX_FEES, &[owner_proof], |max_fee| {
        tx.clone()
            .pay_fee_from_component(account, max_fee)
            .publish_template(template.clone().into_code())
            .build_and_seal(&key)
    });

    assert_only_weight_and_storage_move(&MAX_FEES, &receipts);
}

/// The property a caller actually depends on, asserted as the promise itself rather than as a
/// number: what `required_fees` returns for *any* run must cover *every* other run.
///
/// A dry run meters at whatever `max_fee` the caller submitted, and the submission built from it
/// uses a smaller one, so the estimate has to hold in both directions — asserting it only from the
/// cheapest run would assume the very thing the allowance exists to cover. The burn rate is varied
/// to show that it does not enter the price: the burn is a share of what is paid, not a charge.
#[test]
fn required_fees_covers_a_real_run_at_any_max_fee() {
    // Spans every encoding width a fee above this transaction's cost can take, every residual
    // width, and digit counts from four to nine.
    const MAX_FEES: [u64; 8] = [
        2_000,
        65_535,
        65_536,
        1_000_000,
        100_000_000,
        FUNDED - 60_000,
        FUNDED - 200,
        FUNDED - 10,
    ];

    for rate in [0u16, 500, 2_000, 10_000] {
        let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
        let (account, owner_token, key) = test.create_funded_account();
        test.enable_fees();
        test.set_burn_rate_bps(rate);
        let build = state_transaction(&test, account, &key);
        let receipts = meter_across_max_fees(&mut test, &MAX_FEES, &[owner_token], build);

        let dearest = receipts
            .iter()
            .map(|r| r.total_fees_charged())
            .max()
            .expect("max_fees is not empty");
        for (max_fee, receipt) in MAX_FEES.iter().zip(&receipts) {
            let estimate = receipt.required_fees();
            assert!(
                dearest <= estimate,
                "at {rate} bps, a dry run at max_fee {max_fee} estimates {estimate}, under the {dearest} that some \
                 other max_fee costs"
            );
        }
    }
}

/// The residual term's direction, which is what makes it harmless. Raising `max_fee` narrows the
/// balance left in the fee vault when the byte counter runs, so it can only ever take storage down.
#[test]
fn a_wider_max_fee_never_raises_the_storage_charge() {
    // Ascending, at one encoding width, so only the residual moves and it only narrows.
    const MAX_FEES: [u64; 4] = [65_536, FUNDED - 60_000, FUNDED - 200, FUNDED - 10];

    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (account, owner_token, key) = test.create_funded_account();
    test.enable_fees();
    let build = state_transaction(&test, account, &key);
    let receipts = meter_across_max_fees(&mut test, &MAX_FEES, &[owner_token], build);

    for (pair, fees) in receipts.windows(2).zip(MAX_FEES.windows(2)) {
        assert!(
            pair[1].fee_breakdown().get(FeeSource::Storage) <= pair[0].fee_breakdown().get(FeeSource::Storage),
            "storage rose between max_fee {} and {}",
            fees[0],
            fees[1]
        );
    }
    // The mechanism is live, not merely one-directional by accident.
    assert!(
        receipts[receipts.len() - 1].fee_breakdown().get(FeeSource::Storage) <
            receipts[0].fee_breakdown().get(FeeSource::Storage)
    );
}

fn assert_only_weight_and_storage_move(max_fees: &[u64], receipts: &[FeeReceipt]) {
    let base = &receipts[0];
    for (max_fee, receipt) in max_fees.iter().zip(receipts) {
        for (source, amount) in receipt.fee_breakdown().iter() {
            if matches!(source, FeeSource::TransactionWeight | FeeSource::Storage) {
                continue;
            }
            assert_eq!(
                *amount,
                base.fee_breakdown().get(*source),
                "{source:?} moved between max_fee {} and {max_fee}",
                max_fees[0]
            );
        }
    }
}

/// The digit-count mechanism traced to its source: the event payload the receipt carries holds
/// `max_fee` itself, rendered in decimal.
#[test]
fn the_pay_fee_event_records_max_fee_in_decimal() {
    const MAX_FEE: u64 = 123_456_789;

    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (account, owner_token, key) = test.create_funded_account();
    test.enable_fees();
    let build = state_transaction(&test, account, &key);
    let result = test.try_execute(build(MAX_FEE), vec![owner_token]).unwrap();

    let pay_fee = result
        .finalize
        .events
        .iter()
        .find(|e| e.topic() == "std.vault.pay_fee")
        .expect("pay_fee event");
    assert_eq!(
        pay_fee.get_payload("amount"),
        Some(MAX_FEE.to_string().as_str()),
        "the event records the payment cap, not the fee actually charged"
    );
}

/// The burn is settled over what was paid, so the rate moves nothing the payer is charged: the
/// same transaction meters the same at any rate, and the receipt's burn is the share of the payment.
#[test]
fn the_exhaust_burn_does_not_move_the_charges() {
    const MAX_FEES: [u64; 2] = [65_536, FUNDED - 10];

    let mut charged_at_rate = Vec::new();
    for rate in [0u16, 500, 10_000] {
        let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
        let (account, owner_token, key) = test.create_funded_account();
        test.enable_fees();
        test.set_burn_rate_bps(rate);
        let build = state_transaction(&test, account, &key);
        let receipts = meter_across_max_fees(&mut test, &MAX_FEES, &[owner_token], build);

        for receipt in &receipts {
            assert_eq!(
                receipt.fee_breakdown().get(FeeSource::Reserved),
                0,
                "at {rate} bps the burn must not be charged"
            );
            let expected_burn = u128::from(receipt.total_fees_paid()) * u128::from(rate) / 10_000;
            assert_eq!(u128::from(receipt.exhaust_burn()), expected_burn, "at {rate} bps");
        }
        charged_at_rate.push(receipts.iter().map(|r| r.total_fees_charged()).collect::<Vec<_>>());
    }

    for charged in &charged_at_rate[1..] {
        assert_eq!(charged, &charged_at_rate[0], "the rate must not move what is charged");
    }
}

#[test]
fn template_load_fee_charged_once_per_template_per_transaction() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);

    let (account, owner_token, private_key) = test.create_funded_account();
    let state: ComponentAddress = test.call_function("State", "new", args![], vec![]);

    test.enable_fees();

    // Single State call — establishes the baseline TemplateLoad fee for {Account, State}.
    let single = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account, 2000u64)
            .call_method(state, "set", args![1u32])
            .build_and_seal(&private_key),
        vec![owner_token.clone()],
    );

    // Five State calls — same template touched five extra times. Without dedup, TemplateLoad
    // would scale with call count; with dedup it must match the single-call baseline.
    let many = test.execute_expect_success(
        test.transaction()
            .pay_fee_from_component(account, 2000u64)
            .call_method(state, "set", args![1u32])
            .call_method(state, "set", args![2u32])
            .call_method(state, "set", args![3u32])
            .call_method(state, "get", args![])
            .call_method(state, "get", args![])
            .build_and_seal(&private_key),
        vec![owner_token],
    );

    let template_load = |result: &tari_engine_types::commit_result::FinalizeResult| -> u64 {
        result
            .fee_receipt
            .fee_breakdown()
            .iter()
            .find_map(|(s, amount)| (*s == FeeSource::TemplateLoad).then_some(*amount))
            .expect("TemplateLoad charge present")
    };

    let single_load = template_load(&single.finalize);
    let many_load = template_load(&many.finalize);
    assert!(single_load > 0, "TemplateLoad fee should be non-zero");
    assert_eq!(
        single_load, many_load,
        "TemplateLoad fee must be deduped per (template, transaction); single={single_load} many={many_load}",
    );
}
