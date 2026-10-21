//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Holds `MergedStealthTransferShape::estimate_fee` to what the engine actually charges.
//!
//! The estimate is a second statement of consensus pricing, written so a wallet can settle a
//! transfer's shape without a network round trip. A second statement drifts, and drift downward is
//! the expensive direction: a builder that settles under the real charge submits a transaction that
//! is rejected for underpayment and collects nothing. So every shape here is priced twice — once by
//! the estimator, once by executing it — and the estimate must never come in under. It must also
//! stay close, since the surplus is funds a builder locks and cannot spend.

use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine::fees::FeeTable;
use tari_engine_types::{
    fees::{FeeRates, FeeReceipt},
    stealth::{MergedStealthTransferShape, persisted_utxo_bytes},
};
use tari_ootle_transaction::{Epoch, Transaction};
use tari_template_lib::types::{
    Amount,
    ComponentAddress,
    NonFungibleAddress,
    constants::TARI_TOKEN,
    stealth::StealthTransferStatement,
};
use tari_template_test_tooling::{
    TemplateTest,
    support::stealth::{self, StealthSecretTransferData},
    wallet_crypto::MaskAndValue,
};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

/// The burn every shipped network is configured with. Applied here rather than left at the harness
/// default of zero, so the estimate is checked against a network that splits what it collects: the
/// burn is a share of the payment, so it must move nothing the estimate prices.
const BURN_RATE_BPS: u16 = 500;

/// Enough TARI behind each transfer that a shape's fee is never the binding constraint on it.
const UTXO_VALUE: u64 = 10_000_000;

/// What the transfer reveals to pay its fee. Comfortably above any shape's real charge, so the
/// transaction commits and the receipt records what the shape cost rather than what it could afford.
const REVEALED_FEE: u64 = 5_000_000;

struct Harness {
    test: TemplateTest,
    account: ComponentAddress,
    owner: NonFungibleAddress,
    key: RistrettoSecretKey,
}

fn setup() -> Harness {
    let mut test = TemplateTest::new(CRATE_PATH, Vec::<&str>::new());
    let (account, owner, key) = test.create_funded_account();
    test.enable_auto_add_proofs_from_signers();
    // The harness prices a created substate at 1; the shipped tables price it at 250. Take the
    // shipped figure so the slot premium carries the weight it does on a real network, where an
    // extra output costs the most.
    test.set_fee_table(FeeTable {
        per_substate_create_cost: 250,
        ..test.fee_table().clone()
    });
    Harness {
        test,
        account,
        owner,
        key,
    }
}

/// The rates the harness is executing at, as the estimator takes them. Both sides pricing from one
/// table is the point: what is under test is the formula, not the numbers it is fed.
fn rates(harness: &Harness) -> FeeRates {
    harness.test.fee_table().to_rates()
}

/// Moves `count × UTXO_VALUE` of the account's TARI into stealth UTXOs, so a later transfer has
/// stealth inputs to spend. Runs without fees: this is scaffolding, not a shape under test.
fn mint_utxos(harness: &mut Harness, count: usize) -> StealthSecretTransferData {
    let total = UTXO_VALUE * count as u64;
    let statement = stealth::generate_transfer_data(
        stealth::NO_INPUTS,
        Amount::from(total),
        vec![UTXO_VALUE; count],
        Amount::zero(),
    );

    harness.test.disable_fees();
    let tx = Transaction::builder_localnet(Epoch(1))
        .call_method(harness.account, "withdraw", tari_ootle_transaction::args![
            TARI_TOKEN, total
        ])
        .put_last_instruction_output_on_workspace("input_bucket")
        .stealth_transfer_with_input_bucket(TARI_TOKEN, statement.statement.clone(), "input_bucket")
        .build_and_seal(&harness.key);
    harness.test.execute_expect_success(tx, vec![harness.owner.clone()]);

    statement
}

/// Runs the flow a wallet runs, for a transfer spending `num_inputs` of `minted`'s UTXOs into
/// `num_outputs` stealth outputs: price the shape statically, then reveal exactly that figure and
/// execute. Returns the resulting fee receipt alongside the shape that was priced.
///
/// Revealing the estimate is what puts it under test. A stealth-revealed fee is not refundable, so
/// the transaction either commits — the estimate covered the charge — or is rejected as underpaid,
/// and whatever it did not need is recorded as an overcharge that measures how far over the estimate
/// sat.
fn estimate_then_execute(
    harness: &mut Harness,
    minted: &StealthSecretTransferData,
    num_inputs: usize,
    num_outputs: usize,
) -> (FeeReceipt, MergedStealthTransferShape) {
    harness.test.enable_fees();
    harness.test.set_burn_rate_bps(BURN_RATE_BPS);

    // Neither the weight nor the outputs' stored size follows the amounts a statement carries, only
    // its shape, so a build at a placeholder fee measures both for the real one.
    let probe = build(harness, minted, num_inputs, num_outputs, PLACEHOLDER_FEE);
    let shape = shape_of(&probe, num_inputs, num_outputs);

    let settled = build(
        harness,
        minted,
        num_inputs,
        num_outputs,
        shape.estimate_fee(&rates(harness)),
    );
    assert_eq!(
        shape_of(&settled, num_inputs, num_outputs),
        shape,
        "the fee a statement reveals must not move the shape it is priced at",
    );

    let result = harness.test.execute_expect_success(settled.transaction, vec![]);
    (result.finalize.fee_receipt, shape)
}

/// The shape a build presents to the estimator: counts from the transfer, stored size from the
/// outputs it generated, weight from the sealed transaction.
fn shape_of(build: &Build, num_inputs: usize, num_outputs: usize) -> MergedStealthTransferShape {
    MergedStealthTransferShape {
        num_inputs,
        num_outputs,
        persisted_output_bytes: build
            .statement
            .outputs_statement
            .outputs
            .iter()
            .map(persisted_utxo_bytes)
            .sum(),
        // TARI is the only resource a fee can be paid in, and it carries no view key.
        has_view_key: false,
        transaction_weight: build.transaction.calculate_transaction_weight().as_u64(),
    }
}

struct Build {
    transaction: Transaction,
    statement: StealthTransferStatement,
}

/// Builds the transaction a wallet builds for the common send: one stealth transfer statement in the
/// fee intent, its revealed remainder paid straight to `pay_fee`.
fn build(
    harness: &Harness,
    minted: &StealthSecretTransferData,
    num_inputs: usize,
    num_outputs: usize,
    revealed_fee: u64,
) -> Build {
    let inputs = (0..num_inputs)
        .map(|i| MaskAndValue {
            mask: minted.output_masks[i].clone(),
            value: UTXO_VALUE,
        })
        .collect::<Vec<_>>();

    // Everything the inputs are worth, less the revealed fee, split evenly across the outputs - the
    // last output takes the remainder, standing in for the change a wallet is left with.
    let to_outputs = UTXO_VALUE * num_inputs as u64 - revealed_fee;
    let per_output = to_outputs / num_outputs as u64;
    let mut outputs = vec![per_output; num_outputs];
    outputs[num_outputs - 1] += to_outputs - per_output * num_outputs as u64;

    let transfer = stealth::generate_transfer_data(inputs, Amount::zero(), outputs, Amount::from(revealed_fee));

    let transaction = (0..num_inputs)
        .fold(
            Transaction::builder_localnet(Epoch(1))
                .with_fee_instructions_builder(|builder| {
                    builder
                        .stealth_transfer(TARI_TOKEN, transfer.statement.clone())
                        .put_last_instruction_output_on_workspace("revealed_bucket")
                        .pay_fee_from_bucket("revealed_bucket")
                })
                .finish(),
            |tx, i| tx.add_signer(&harness.test.to_public_key_bytes(), &minted.output_masks[i]),
        )
        .seal(harness.test.secret_key());

    Build {
        transaction,
        statement: transfer.statement,
    }
}

/// Stands in for the fee while the statement is built only to be weighed. Any figure the inputs
/// cover gives the same weight.
const PLACEHOLDER_FEE: u64 = 1_000_000;

/// The margin the estimate may carry over the real charge. Measuring the outputs a build generated
/// leaves the receipt's epoch as the only field still priced at a stand-in width, so what remains is
/// a handful of microtari — and it is paid and not refunded, so it has to stay that way.
const MAX_OVERSHOOT: u64 = 12;

fn assert_bounds(receipt: &FeeReceipt, shape: &MergedStealthTransferShape) {
    // Reaching here at all is the lower bound: an estimate under the charge leaves an unpaid debt,
    // and `execute_expect_success` would have failed on the rejection.
    assert!(
        receipt.is_paid_in_full(),
        "{shape:?} left {} unpaid",
        receipt.unpaid_debt()
    );

    let overshoot = receipt.total_fee_overcharge();
    assert!(
        overshoot <= MAX_OVERSHOOT,
        "{shape:?} was charged {} and overpaid {overshoot}, which it cannot get back",
        receipt.total_fees_charged(),
    );
}

/// The whole matrix in one test, over one harness: bootstrapping a state store is the expensive
/// part, and each case only needs UTXOs of its own to spend.
#[test]
fn the_estimate_bounds_what_the_engine_charges() {
    let mut harness = setup();
    for num_inputs in 1..=4usize {
        for num_outputs in 1..=3usize {
            let minted = mint_utxos(&mut harness, num_inputs);
            let (receipt, shape) = estimate_then_execute(&mut harness, &minted, num_inputs, num_outputs);
            assert_bounds(&receipt, &shape);
        }
    }
}

/// The estimate's host-call count is a constant standing in for an instruction sequence it cannot
/// see. Reading the charge back at a known per-call rate pins it to what the engine counted.
#[test]
fn the_runtime_call_count_matches_the_instruction_sequence() {
    use tari_engine::fees::FeeTable;
    use tari_engine_types::fees::FeeSource;

    let mut harness = setup();
    let minted = mint_utxos(&mut harness, 1);

    // Only host calls are priced, so the charge under `RuntimeCall` is the call count itself.
    let mut fee_table = FeeTable::zero_rated();
    fee_table.per_module_call_cost = 1;
    harness.test.set_fee_table(fee_table);
    harness.test.set_burn_rate_bps(0);

    let inputs = vec![MaskAndValue {
        mask: minted.output_masks[0].clone(),
        value: UTXO_VALUE,
    }];
    let transfer = stealth::generate_transfer_data(
        inputs,
        Amount::zero(),
        vec![UTXO_VALUE - REVEALED_FEE],
        Amount::from(REVEALED_FEE),
    );

    harness.test.enable_fees();
    let tx = Transaction::builder_localnet(Epoch(1))
        .with_fee_instructions_builder(|builder| {
            builder
                .stealth_transfer(TARI_TOKEN, transfer.statement.clone())
                .put_last_instruction_output_on_workspace("revealed_bucket")
                .pay_fee_from_bucket("revealed_bucket")
        })
        .finish()
        .add_signer(&harness.test.to_public_key_bytes(), &minted.output_masks[0])
        .seal(harness.test.secret_key());

    let result = harness.test.execute_expect_success(tx, vec![]);
    assert_eq!(
        result.finalize.fee_receipt.fee_breakdown().get(FeeSource::RuntimeCall),
        MergedStealthTransferShape::RUNTIME_CALLS,
    );
}

/// The matrix's lower bound rests on an underpaid transaction being rejected. Revealing a hair under
/// what the shape costs shows that it is: without this, an estimate that came in under would still
/// commit and the matrix would be asserting nothing.
#[test]
fn revealing_under_the_charge_is_rejected() {
    let mut harness = setup();
    let minted = mint_utxos(&mut harness, 1);
    harness.test.enable_fees();
    harness.test.set_burn_rate_bps(BURN_RATE_BPS);

    let shape = shape_of(&build(&harness, &minted, 1, 1, PLACEHOLDER_FEE), 1, 1);

    // Half the estimate is under the charge whatever the estimate's margin, which MAX_OVERSHOOT
    // bounds to a few microtari.
    let too_little = shape.estimate_fee(&rates(&harness)) / 2;
    let result = harness
        .test
        .try_execute(build(&harness, &minted, 1, 1, too_little).transaction, vec![])
        .unwrap();

    assert!(
        result.finalize.fee_receipt.unpaid_debt() > 0,
        "revealing {too_little} should have left a debt",
    );
    result.expect_failure();
}
