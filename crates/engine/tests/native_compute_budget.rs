//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Native verification (stealth transfers, confidential withdraws, burn claims) is priced in
//! WASM-point equivalents and charged against the same payment-funded allowance as WASM execution,
//! *before* the crypto runs. These tests prove the three load-bearing properties:
//!
//! 1. the grace covers every legitimate fee-sourcing flow (sizing relation over the constants);
//! 2. a transaction that does not pay is rejected before performing native verification, so garbage proofs cannot
//!    extract crypto work from a validator;
//! 3. a paying transaction is charged for its native verification under `FeeSource::NativeExecution`.

use ootle_byte_type::ToByteType;
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey};
use tari_engine::fees::FeeTable;
use tari_engine_types::{
    fees::FeeSource,
    limits::{FREE_COMPUTE_GRACE_POINTS, MAX_NATIVE_POINTS_PER_TRANSACTION, NativeExecutionPoints, STEALTH_LIMITS},
};
use tari_ootle_common_types::substate_type::SubstateType;
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::types::{ComponentAddress, NonFungibleAddress, ResourceAddress};
use tari_template_test_tooling::{
    TemplateTest,
    support::{assert_error::assert_reject_reason, stealth, stealth::StealthSecretTransferData},
    wallet_crypto::MaskAndValue,
};

const TEMPLATE_PATHS: &[&str] = &["tests/templates/stealth"];
const TEMPLATE_NAME: &str = "StealthFaucet";
const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

/// [`MAX_NATIVE_POINTS_PER_TRANSACTION`] must sit above the most expensive statement set
/// [`STEALTH_LIMITS`] admits, or a transaction the structural caps accept traps on the ceiling
/// instead of executing. The two move for unrelated reasons — the caps bound one transaction's
/// verification, the ceiling bounds how far a block may overshoot its propose-time execution
/// budget — so the relation between them is asserted rather than assumed.
///
/// Priced at the view-key rate, the dearest an output can be. A transaction that stacks *several*
/// categories at their individual maxima (a full stealth set plus a full confidential set) can
/// still exceed the ceiling and trap; that is intended, since it is ~1s of verification in a single
/// transaction and no real flow needs it.
#[test]
fn native_ceiling_admits_every_structurally_valid_stealth_transaction() {
    let per_output = NativeExecutionPoints::PER_OUTPUT + NativeExecutionPoints::PER_OUTPUT_VIEWABLE_SURCHARGE;
    let worst_case = STEALTH_LIMITS.max_transfers_per_transaction as u64 * NativeExecutionPoints::PER_STATEMENT +
        STEALTH_LIMITS.max_total_outputs_per_transaction as u64 * per_output +
        STEALTH_LIMITS.max_total_inputs_per_transaction as u64 * NativeExecutionPoints::PER_INPUT;

    assert!(
        worst_case <= MAX_NATIVE_POINTS_PER_TRANSACTION,
        "MAX_NATIVE_POINTS_PER_TRANSACTION ({MAX_NATIVE_POINTS_PER_TRANSACTION}) must admit the dearest structurally \
         valid stealth transaction ({worst_case} points); raise it or tighten STEALTH_LIMITS",
    );
}

/// The grace must cover every legitimate fee-sourcing flow with margin, since all of it runs on
/// credit before `pay_fee`. The stealth-UTXO-funded fee is the ceiling: one transfer statement
/// with a single stealth change output and up to 64 dust inputs. The AMM-swap (WASM) flow is
/// guarded end-to-end by `complex_fee_payment`.
#[test]
#[allow(clippy::assertions_on_constants)]
fn grace_covers_legitimate_fee_sourcing_flows() {
    const SAFETY_FACTOR: u64 = 2;
    const FEE_SOURCE_DUST_INPUTS: u64 = 64;

    let stealth_fee_source = NativeExecutionPoints::PER_STATEMENT +
        NativeExecutionPoints::PER_OUTPUT +
        FEE_SOURCE_DUST_INPUTS * NativeExecutionPoints::PER_INPUT;
    assert!(
        stealth_fee_source * SAFETY_FACTOR <= FREE_COMPUTE_GRACE_POINTS,
        "FREE_COMPUTE_GRACE_POINTS ({FREE_COMPUTE_GRACE_POINTS}) must stay at least {SAFETY_FACTOR}x above the \
         stealth fee-sourcing flow ({stealth_fee_source} points)",
    );

    assert!(
        NativeExecutionPoints::PER_CLAIM_BURN * SAFETY_FACTOR <= FREE_COMPUTE_GRACE_POINTS,
        "FREE_COMPUTE_GRACE_POINTS ({FREE_COMPUTE_GRACE_POINTS}) must stay at least {SAFETY_FACTOR}x above a burn \
         claim ({} points)",
        NativeExecutionPoints::PER_CLAIM_BURN,
    );
}

fn setup_faucet(
    test: &mut TemplateTest,
    transfer_data: &StealthSecretTransferData,
    view_key: Option<&RistrettoPublicKey>,
) -> (ComponentAddress, ResourceAddress) {
    test.enable_auto_add_proofs_from_signers();
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let initial_supply = transfer_data.statement.inputs_statement.revealed_amount;

    let transaction = Transaction::builder_localnet(Epoch(1))
        .call_function(template_addr, "new", args![
            initial_supply,
            transfer_data.statement,
            view_key.map(|vk| vk.to_byte_type())
        ])
        .build_and_seal(test.secret_key());

    test.execute_expect_success(transaction, vec![]);

    let faucet = test.get_previous_output_address(SubstateType::Component);
    let resx = test.get_previous_output_address(SubstateType::Resource);

    (
        faucet.as_component_address().unwrap(),
        resx.as_resource_address().unwrap(),
    )
}

fn enable_point_priced_fees(test: &mut TemplateTest) {
    // 1 fee unit per metering point so charges equal points and small payments isolate the budget.
    let mut fee_table = FeeTable::zero_rated();
    fee_table.per_wasm_point_cost = 1;
    fee_table.wasm_points_cost_divisor = 1;
    test.set_fee_table(fee_table);
    test.enable_fees();
}

/// A transaction whose fee intent carries a statement priced above the credit is rejected by the
/// allowance pre-charge — before any of its crypto runs. The statement's range proof is
/// deliberately corrupted: were the verification performed, the failure would be a range-proof
/// error, so the credit rejection proves the charge fired first.
#[test]
fn unpaid_native_verification_traps_before_the_crypto_runs() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let mint = stealth::generate_mint_statement([100, 1000], 0u64, None);
    let (_faucet, faucet_resx) = setup_faucet(&mut test, &mint, None);
    enable_point_priced_fees(&mut test);

    // 8 outputs price above the 32M grace (fixed + 8 × per-output ≈ 50M points).
    let mut garbage = stealth::generate_mint_statement(vec![100u64; 8], 0u64, None);
    let mut rp = garbage.statement.outputs_statement.agg_range_proof.clone().into_vec();
    rp[100] ^= 0xFF;
    garbage.statement.outputs_statement.agg_range_proof = rp.try_into().unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| builder.stealth_transfer(faucet_resx, garbage.statement))
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "points of compute credit");
}

/// Paying first does not buy more native verification inside the fee intent. The credit is flat, so
/// a payment that would fund the statement several times over past the checkpoint still leaves it
/// rejected by the pre-charge — with the same corrupted proof, so the rejection again proves the
/// charge fired before any crypto ran.
#[test]
fn paying_first_does_not_raise_the_fee_intents_native_allowance() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let mint = stealth::generate_mint_statement([100, 1000], 0u64, None);
    let (_faucet, faucet_resx) = setup_faucet(&mut test, &mint, None);
    let (account, owner, key) = test.create_funded_account();
    enable_point_priced_fees(&mut test);

    // 8 outputs price above the 32M credit (fixed + 8 × per-output ≈ 50M points).
    let mut garbage = stealth::generate_mint_statement(vec![100u64; 8], 0u64, None);
    let mut rp = garbage.statement.outputs_statement.agg_range_proof.clone().into_vec();
    rp[100] ^= 0xFF;
    garbage.statement.outputs_statement.agg_range_proof = rp.try_into().unwrap();

    // 1 fee unit per point, so this funds the statement's ~50M points four times over.
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| {
                builder
                    .pay_fee_from_component(account, 200_000_000u64)
                    .stealth_transfer(faucet_resx, garbage.statement)
            })
            .build_and_seal(&key),
        vec![owner],
    );

    assert_reject_reason(reason, "points of compute credit");
}

/// Revealed-only statements (no stealth/confidential inputs or outputs) short-circuit every
/// verifier — no balance proof, no range proof — so they must price at zero: free-coins claims and
/// revealed→revealed transfers keep their pre-metering fees.
#[test]
fn revealed_only_statements_price_at_zero() {
    use tari_template_lib::types::{confidential::ConfidentialWithdrawProof, crypto::PedersenCommitmentBytes};

    let revealed_transfer = stealth::generate_transfer_data(stealth::NO_INPUTS, 100u64, Vec::<u64>::new(), 100u64);
    assert_eq!(
        tari_engine_types::stealth::transfer_native_points(&revealed_transfer.statement, false),
        0
    );

    let revealed_withdraw = ConfidentialWithdrawProof::revealed_withdraw(1000u64);
    assert_eq!(
        tari_engine_types::confidential::withdraw_native_points(&revealed_withdraw, false),
        0
    );
    assert_eq!(
        tari_engine_types::confidential::statement_native_points(&revealed_withdraw.output_proof, false),
        0
    );

    // Any input brings the balance-proof verification: a withdraw-everything-to-revealed proof
    // (inputs, no outputs) is not free.
    let mut with_input = ConfidentialWithdrawProof::revealed_withdraw(1000u64);
    with_input.inputs.push(PedersenCommitmentBytes::zero());
    assert_eq!(
        tari_engine_types::confidential::withdraw_native_points(&with_input, false),
        NativeExecutionPoints::PER_STATEMENT + NativeExecutionPoints::PER_INPUT
    );
}

/// WASM consumed in-flight counts toward the allowance a mid-invocation native charge is checked
/// against. The template grinds most of the grace away and only then triggers a stealth transfer
/// (a host call) whose native price would fit the allowance if the in-flight consumption were
/// invisible — the charge must see it and trap. Guards the meter sync at host-call entry: without
/// it, a transaction could spend the whole grace on WASM and still extract native verification on
/// top for free.
#[test]
fn in_flight_wasm_counts_toward_the_native_allowance() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    // Revealed output amount funds the faucet's supply vault, which the programmatic transfer
    // draws its revealed input from.
    let mint = stealth::generate_mint_statement([100, 1000], 200u64, None);
    let (faucet, _faucet_resx) = setup_faucet(&mut test, &mint, None);
    let (account, owner, key) = test.create_funded_account();
    enable_point_priced_fees(&mut test);

    // Calibrate points-per-round with a paid transaction.
    let points_for = |test: &mut TemplateTest, rounds: u64| -> u64 {
        let tx = Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account, 100_000_000u64)
            .call_method(faucet, "burn_compute", args![rounds])
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
    let per_round = (points_for(&mut test, 2_000) - points_for(&mut test, 1_000)) / 1_000;

    // A revealed-funded single-output transfer: ~10.1M native points. Grind enough that the
    // in-flight WASM plus the native price exceeds the grace, while each alone fits within it.
    let transfer = stealth::generate_transfer_data(stealth::NO_INPUTS, 100u64, [50], 50u64);
    let native_points = tari_engine_types::stealth::transfer_native_points(&transfer.statement, false);
    assert!(native_points < FREE_COMPUTE_GRACE_POINTS);
    let grind_points = FREE_COMPUTE_GRACE_POINTS - native_points / 2;
    let rounds = grind_points / per_round;

    // Runs in the fee intent, which is where the credit applies — the main instructions are funded by
    // the payment alone and would trap on the WASM grind long before the native charge.
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| {
                builder.call_method(faucet, "burn_compute_then_transfer", args![rounds, transfer.statement])
            })
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "points of compute credit");
}

/// The `NativeExecution` charge a bare fee payment carries: `pay_fee_from_component` calls into the
/// Account template, and every call instantiates its template. Subtracting it leaves the charge for
/// the verification the test is actually about.
fn fee_payment_native_charge(test: &mut TemplateTest, account: ComponentAddress, owner: NonFungibleAddress) -> u64 {
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account, 900_000_000u64)
            .build_and_seal(test.secret_key()),
        vec![owner],
    );
    native_charge(&result)
}

fn native_charge(result: &tari_engine_types::commit_result::ExecuteResult) -> u64 {
    result
        .finalize
        .fee_receipt
        .fee_breakdown()
        .iter()
        .find_map(|(s, a)| (*s == FeeSource::NativeExecution).then_some(*a))
        .expect("NativeExecution charge present")
}

/// A paying transaction's native verification is charged under `FeeSource::NativeExecution` at the
/// per-point rate.
#[test]
fn paid_native_verification_is_charged() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let mint = stealth::generate_mint_statement([100, 1000], 0u64, None);
    let (_faucet, faucet_resx) = setup_faucet(&mut test, &mint, None);
    let (account, owner, key) = test.create_funded_account();
    enable_point_priced_fees(&mut test);

    let transfer = stealth::generate_transfer_data(
        [MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        }],
        0u64,
        Some(100),
        0,
    );
    let expected_points = tari_engine_types::stealth::transfer_native_points(&transfer.statement, false);
    let baseline = fee_payment_native_charge(&mut test, account, owner.clone());

    let seal_signer = RistrettoPublicKey::from_secret_key(&key).to_byte_type();
    // Explicit proofs suppress the tooling's auto-added signer badges, so the UTXO spend key's
    // badge must be supplied alongside the fee account's owner badge.
    let mask_badge =
        NonFungibleAddress::from_public_key(RistrettoPublicKey::from_secret_key(&mint.output_masks[0]).to_byte_type());
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account, 900_000_000u64)
            .stealth_transfer(faucet_resx, transfer.statement)
            .finish()
            .add_signer(&seal_signer, &mint.output_masks[0])
            .seal(&key),
        vec![owner, mask_badge],
    );

    assert_eq!(native_charge(&result) - baseline, expected_points);
}

/// A resource with a view key verifies an ElGamal viewable-balance proof per output, so its
/// transfers are charged the per-output surcharge; TARI never carries a view key, so fee-sourcing
/// transfers always price at the base rate.
#[test]
fn view_key_surcharge_is_charged_per_output() {
    use tari_crypto::{keys::SecretKey, ristretto::RistrettoSecretKey};

    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let view_key_secret = RistrettoSecretKey::random(&mut rand::rng());
    let view_key = RistrettoPublicKey::from_secret_key(&view_key_secret);
    let mint = stealth::generate_mint_statement([100, 1000], 0u64, Some(&view_key));
    let (_faucet, faucet_resx) = setup_faucet(&mut test, &mint, Some(&view_key));
    let (account, owner, key) = test.create_funded_account();
    enable_point_priced_fees(&mut test);

    let transfer = stealth::generate_transfer_data_with_view_key(
        [MaskAndValue {
            mask: mint.output_masks[0].clone(),
            value: 100,
        }],
        0u64,
        Some(100),
        0u64,
        &view_key,
    );
    let baseline = fee_payment_native_charge(&mut test, account, owner.clone());
    let expected_points = tari_engine_types::stealth::transfer_native_points(&transfer.statement, true);
    assert_eq!(
        expected_points,
        tari_engine_types::stealth::transfer_native_points(&transfer.statement, false) +
            NativeExecutionPoints::PER_OUTPUT_VIEWABLE_SURCHARGE,
        "one output => exactly one surcharge",
    );

    let seal_signer = RistrettoPublicKey::from_secret_key(&key).to_byte_type();
    let mask_badge =
        NonFungibleAddress::from_public_key(RistrettoPublicKey::from_secret_key(&mint.output_masks[0]).to_byte_type());
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account, 900_000_000u64)
            .stealth_transfer(faucet_resx, transfer.statement)
            .finish()
            .add_signer(&seal_signer, &mint.output_masks[0])
            .seal(&key),
        vec![owner, mask_badge],
    );

    assert_eq!(native_charge(&result) - baseline, expected_points);
}
