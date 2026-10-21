//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! The per-transaction WASM compute budget is bounded by the fees paid, except in the fee intent,
//! which runs on a flat `FREE_COMPUTE_GRACE_POINTS` of credit — enough to source its fee (withdraw,
//! claim-burn, AMM swap, …) before it calls `pay_fee`. Past the fee checkpoint each WASM call's
//! metering allowance is capped to the points the fees paid so far can cover. These tests prove a
//! transaction cannot extract more than the credit of unpaid compute, that paying more raises the
//! allowance in the main instructions but not in the fee intent, and that fee-sourcing compute
//! within the credit is allowed.

use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine::fees::FeeTable;
use tari_engine_types::{
    commit_result::{ExecutionFailureCode, RejectReason, TransactionResult},
    fees::FeeSource,
    limits::FREE_COMPUTE_GRACE_POINTS,
};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::types::{ComponentAddress, NonFungibleAddress, TemplateAddress};
use tari_template_test_tooling::TemplateTest;

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");
const METERING_BENCH: &str = "tests/templates/metering_bench";

/// A call sized above the grace (and above `grace + the tests' fee payment`, so it still traps when
/// a tiny fee is paid) but well under `MAX_WASM_POINTS_PER_TRANSACTION`, so the per-transaction hard
/// cap is never the binding constraint — only the payment-funded allowance is.
const ABOVE_GRACE_POINTS: u64 = FREE_COMPUTE_GRACE_POINTS * 2;
/// A call sized below the grace, runnable on credit before any fee is paid.
const BELOW_GRACE_POINTS: u64 = FREE_COMPUTE_GRACE_POINTS / 2;

struct Harness {
    test: TemplateTest,
    bench: TemplateAddress,
    account: ComponentAddress,
    owner: NonFungibleAddress,
    key: RistrettoSecretKey,
    per_round: u64,
}

/// Builds a test where only WASM execution is priced (1 fee unit per metering point) so a
/// transaction's fee charge equals the points it consumed and a small payment isolates the compute
/// budget, then calibrates points-per-round so calls can be sized to a known multiple of the grace.
fn setup() -> Harness {
    let mut test = TemplateTest::new(CRATE_PATH, [METERING_BENCH]);
    let bench = test.get_template_address("MeteringBench");
    let (account, owner, key) = test.create_funded_account();

    let mut fee_table = FeeTable::zero_rated();
    fee_table.per_wasm_point_cost = 1;
    fee_table.wasm_points_cost_divisor = 1;
    test.set_fee_table(fee_table);
    test.enable_fees();

    let per_round = {
        let mut points = |rounds: u64| -> u64 {
            let tx = Transaction::builder_localnet(Epoch(1))
                .pay_fee_from_component(account, 900_000_000u64)
                .call_function(bench, "bench_div_u64", args![rounds])
                .build_and_seal(&key);
            let result = test.execute_expect_success(tx, vec![owner.clone()]);
            result
                .finalize
                .fee_receipt
                .fee_breakdown()
                .iter()
                .find_map(|(s, a)| (*s == FeeSource::WasmExecution).then_some(*a))
                .expect("WasmExecution charge present")
        };
        (points(20_000) - points(10_000)) / 10_000
    };

    Harness {
        test,
        bench,
        account,
        owner,
        key,
        per_round,
    }
}

fn assert_insufficient_fees(reason: &RejectReason) {
    assert!(
        matches!(reason, RejectReason::InsufficientFeesPaid(_)),
        "expected an insufficient-fees-for-compute failure, got {reason:?}",
    );
}

fn assert_fee_intent_credit_exceeded(reason: &RejectReason) {
    assert!(
        matches!(
            reason,
            RejectReason::ExecutionFailure {
                code: ExecutionFailureCode::LimitExceeded,
                message,
            } if message.contains("compute credit")
        ),
        "expected the fee intent's compute credit to be the binding limit, got {reason:?}",
    );
}

/// A transaction that pays only a tiny fee cannot run more compute than that fee funds, even though
/// the call is well under the per-transaction hard cap and would otherwise succeed. The fee intent
/// still commits and the whole payment is collected, and because the credit does not extend past the
/// fee checkpoint the validator is paid for every point it ran — there is no absorbed loss.
#[test]
fn underpaid_compute_is_capped_at_what_the_payment_funds() {
    const FEE_PAYMENT: u64 = 1_000_000;

    let Harness {
        mut test,
        bench,
        account,
        owner,
        key,
        per_round,
    } = setup();

    let rounds = ABOVE_GRACE_POINTS / per_round;
    let tx = Transaction::builder_localnet(Epoch(1))
        .pay_fee_from_component(account, FEE_PAYMENT)
        .call_function(bench, "bench_div_u64", args![rounds])
        .build_and_seal(&key);

    let result = test.try_execute(tx, vec![owner]).unwrap();
    assert_insufficient_fees(result.expect_failure());

    assert!(
        result.finalize.is_fee_only(),
        "expected the fee intent to commit and the rest to be rejected (AcceptFeeRejectRest)",
    );
    let fee_receipt = &result.finalize.fee_receipt;
    assert_eq!(
        fee_receipt.total_fees_paid(),
        FEE_PAYMENT,
        "the entire fee payment should be collected",
    );
    assert_eq!(
        fee_receipt.total_refunded(),
        0,
        "nothing is refunded when compute exceeds the payment"
    );
    assert_eq!(
        fee_receipt.unpaid_debt(),
        0,
        "the allowance is exactly what the payment funds, so no compute runs unpaid",
    );
}

/// Nested cross-template calls draw on the same payment-funded allowance as the frame that made
/// them. Each of the two frames here fits the payment on its own but not together, so the second
/// to finish must trap: the fee intent commits and every point that ran is paid for.
#[test]
fn nested_calls_share_the_payment_funded_allowance() {
    let Harness {
        mut test,
        bench,
        account,
        owner,
        key,
        per_round,
    } = setup();

    let rounds = ABOVE_GRACE_POINTS / per_round;
    // What `paying_more_raises_the_compute_allowance` proves funds one frame of this size.
    let fee_payment = ABOVE_GRACE_POINTS + 10_000_000;
    let tx = Transaction::builder_localnet(Epoch(1))
        .pay_fee_from_component(account, fee_payment)
        .call_function(bench, "nested_div_u64", args![bench, 1u32, rounds])
        .build_and_seal(&key);

    let result = test.try_execute(tx, vec![owner]).unwrap();
    assert_insufficient_fees(result.expect_failure());
    assert!(
        result.finalize.is_fee_only(),
        "expected the fee intent to commit and the rest to be rejected (AcceptFeeRejectRest), got {:?}",
        result.finalize.result,
    );
    let fee_receipt = &result.finalize.fee_receipt;
    assert_eq!(fee_receipt.total_fees_paid(), fee_payment);
    assert_eq!(
        fee_receipt.unpaid_debt(),
        0,
        "nested frames must not run compute the payment does not fund",
    );
}

/// The credit is scoped to the fee intent. A call sized within the grace — and which therefore
/// succeeds inside a fee intent, per `fee_intent_may_spend_compute_within_grace_before_paying` —
/// still traps when it runs in the main instructions of a transaction whose payment does not fund
/// it. Without this scoping every transaction would carry `FREE_COMPUTE_GRACE_POINTS` of compute it
/// never pays for, on top of what it bought.
#[test]
fn grace_does_not_extend_past_the_fee_checkpoint() {
    const FEE_PAYMENT: u64 = 1_000_000;
    // The payment must fund materially less than the call needs, so the credit is what the call
    // would otherwise be leaning on.
    const _: () = assert!(BELOW_GRACE_POINTS > FEE_PAYMENT * 2);

    let Harness {
        mut test,
        bench,
        account,
        owner,
        key,
        per_round,
    } = setup();

    let rounds = BELOW_GRACE_POINTS / per_round;
    let tx = Transaction::builder_localnet(Epoch(1))
        .pay_fee_from_component(account, FEE_PAYMENT)
        .call_function(bench, "bench_div_u64", args![rounds])
        .build_and_seal(&key);

    let result = test.try_execute(tx, vec![owner]).unwrap();
    assert_insufficient_fees(result.expect_failure());
}

/// Paying more fees raises the compute allowance: the same call that fails when underpaid commits
/// when the fee funds compute beyond the grace.
#[test]
fn paying_more_raises_the_compute_allowance() {
    let Harness {
        mut test,
        bench,
        account,
        owner,
        key,
        per_round,
    } = setup();

    let rounds = ABOVE_GRACE_POINTS / per_round;
    // Sized from the call's point cost (1 fee unit per point) plus margin for the fixed charges,
    // so it keeps funding the call as the grace constant moves.
    let fee_payment = ABOVE_GRACE_POINTS + 10_000_000;
    let tx = Transaction::builder_localnet(Epoch(1))
        .pay_fee_from_component(account, fee_payment)
        .call_function(bench, "bench_div_u64", args![rounds])
        .build_and_seal(&key);

    test.execute_expect_success(tx, vec![owner]);
}

/// The fee intent may spend real compute to source its fee before it pays, as long as it stays within
/// the grace.
#[test]
fn fee_intent_may_spend_compute_within_grace_before_paying() {
    let Harness {
        mut test,
        bench,
        account,
        owner,
        key,
        per_round,
    } = setup();

    let rounds = BELOW_GRACE_POINTS / per_round;
    let tx = Transaction::builder_localnet(Epoch(1))
        .with_fee_instructions_builder(|builder| {
            builder
                .call_function(bench, "bench_div_u64", args![rounds])
                .pay_fee_from_component(account, 20_000_000u64)
        })
        .build_and_seal(&key);

    test.execute_expect_success(tx, vec![owner]);
}

/// A fee intent that tries to run more than the grace of compute before paying traps out-of-gas and
/// the whole transaction is rejected — bounding the free compute extractable by stuffing an unbounded
/// loop into the fee intent.
#[test]
fn fee_intent_cannot_exceed_grace_compute() {
    let Harness {
        mut test,
        bench,
        account,
        owner,
        key,
        per_round,
    } = setup();

    let rounds = ABOVE_GRACE_POINTS / per_round;
    let tx = Transaction::builder_localnet(Epoch(1))
        .with_fee_instructions_builder(|builder| {
            builder
                .call_function(bench, "bench_div_u64", args![rounds])
                .pay_fee_from_component(account, 50_000_000u64)
        })
        .build_and_seal(&key);

    let reason = test.execute_expect_failure(tx, vec![owner]);
    assert_fee_intent_credit_exceeded(&reason);
}

/// The fee intent's credit is flat: paying first does not buy more compute inside it. A payment
/// that comfortably funds the call in the main instructions still leaves it trapped when the call
/// runs in the fee intent.
///
/// Without this, the fee intent would be the only place worth executing anything. A failure there
/// leaves no checkpoint to fall back to, so the transaction settles as a rejection that collects
/// nothing — the same work in the main instructions falls back to a fee-intent commit and is paid
/// for.
#[test]
fn paying_first_does_not_raise_the_fee_intents_allowance() {
    let Harness {
        mut test,
        bench,
        account,
        owner,
        key,
        per_round,
    } = setup();

    let rounds = ABOVE_GRACE_POINTS / per_round;
    // The payment `paying_more_raises_the_compute_allowance` proves is enough to run this call in
    // the main instructions.
    let fee_payment = ABOVE_GRACE_POINTS + 10_000_000;
    let tx = Transaction::builder_localnet(Epoch(1))
        .with_fee_instructions_builder(|builder| {
            builder
                .pay_fee_from_component(account, fee_payment)
                .call_function(bench, "bench_div_u64", args![rounds])
        })
        .build_and_seal(&key);

    let reason = test.execute_expect_failure(tx, vec![owner]);
    assert_fee_intent_credit_exceeded(&reason);
}

/// The credit binds a dry run as it binds a real run, so an above-credit fee intent fails
/// estimation instead of first appearing at submission — where the sender has no remediation
/// beyond restructuring the transaction.
#[test]
fn a_dry_run_is_bound_by_the_fee_intents_credit() {
    let Harness {
        mut test,
        bench,
        account,
        owner,
        key,
        per_round,
    } = setup();

    let rounds = ABOVE_GRACE_POINTS / per_round;
    let tx = Transaction::builder_localnet(Epoch(1))
        .with_fee_instructions_builder(|builder| {
            builder
                .call_function(bench, "bench_div_u64", args![rounds])
                .pay_fee_from_component(account, 50_000_000u64)
        })
        .build_and_seal(&key);

    test.set_dry_run(true);
    let reason = test.execute_expect_failure(tx, vec![owner]);
    assert_fee_intent_credit_exceeded(&reason);
}

/// Host-only instructions after the last WASM call are charged outside every invocation's
/// allowance, so a main intent that spends its allowance and then runs them can finish a few
/// microtari past the payment. The fee intent paid for its own commit, so that commit still settles
/// and collects the whole payment; the shortfall is no reason to refund the work that already ran.
#[test]
fn charges_after_the_last_invocation_still_collect_the_payment() {
    const TRAILING_HOST_CALLS: u64 = 8;

    let Harness {
        mut test,
        bench,
        account,
        owner,
        key,
        per_round,
    } = setup();
    let mut fee_table = test.fee_table().clone();
    fee_table.per_module_call_cost = 1;
    test.set_fee_table(fee_table);

    let rounds = ABOVE_GRACE_POINTS / per_round;
    let tx = |payment: u64| {
        let mut builder = Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account, payment)
            .call_function(bench, "bench_div_u64", args![rounds]);
        for _ in 0..TRAILING_HOST_CALLS {
            builder = builder.drop_all_proofs_in_workspace();
        }
        builder.build_and_seal(&key)
    };

    // Both payments encode at the same width, so the two runs meter identically.
    let funded = test.execute_expect_success(tx(ABOVE_GRACE_POINTS * 2), vec![owner.clone()]);
    let charged = funded.finalize.fee_receipt.total_fees_charged();

    // Short by less than the trailing calls cost: the WASM call still fits its allowance and
    // finishes, and only the trailing calls take the total past the payment.
    let payment = charged - TRAILING_HOST_CALLS / 2;
    let result = test.try_execute(tx(payment), vec![owner]).unwrap();
    assert_eq!(
        result.wasm_execution_points, funded.wasm_execution_points,
        "the WASM call must run to completion"
    );

    assert!(
        result.finalize.is_fee_only(),
        "expected the fee intent to commit and the rest to be rejected (AcceptFeeRejectRest), got {:?}",
        result.finalize.result,
    );
    assert_insufficient_fees(result.expect_failure());
    let fee_receipt = &result.finalize.fee_receipt;
    assert_eq!(fee_receipt.total_fees_paid(), payment, "the whole payment is collected");
    assert_eq!(
        fee_receipt.total_fees_charged(),
        charged,
        "the receipt reports what was metered"
    );
    assert_eq!(fee_receipt.unpaid_debt(), TRAILING_HOST_CALLS / 2);
}

/// A fee intent whose own commit costs more than it paid still commits nothing, whatever the main
/// intent did. Its execution is priced only at finalization, so a payment can clear the checkpoint
/// and still fall short of what the fee intent ran.
#[test]
fn a_fee_intent_that_cannot_pay_for_itself_commits_nothing() {
    // Clears the checkpoint, which prices no execution under this fee table, but not the points the
    // fee intent's own call ran.
    const FEE_PAYMENT: u64 = 1_000;

    let Harness {
        mut test,
        bench,
        account,
        owner,
        key,
        per_round,
    } = setup();

    let rounds = BELOW_GRACE_POINTS / per_round;
    let tx = Transaction::builder_localnet(Epoch(1))
        .with_fee_instructions_builder(|builder| {
            builder
                .call_function(bench, "bench_div_u64", args![rounds])
                .pay_fee_from_component(account, FEE_PAYMENT)
        })
        .drop_all_proofs_in_workspace()
        .build_and_seal(&key);

    let result = test.try_execute(tx, vec![owner]).unwrap();
    assert!(
        matches!(
            result.finalize.result,
            TransactionResult::Reject(RejectReason::InsufficientFeesPaid(_))
        ),
        "expected the whole transaction to be rejected, got {:?}",
        result.finalize.result,
    );
    assert_eq!(result.finalize.fee_receipt.total_fees_paid(), 0);
    assert!(result.finalize.fee_receipt.total_fees_charged() > FEE_PAYMENT);
}
