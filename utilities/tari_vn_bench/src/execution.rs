//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Transaction execution throughput, measured through the real engine.
//!
//! This is the phase the whole tool exists for. A validator executes a block's commands serially,
//! inside one view, and a machine that cannot finish in time misses the proposal — five of those
//! and it is suspended. Everything measured here feeds one question: how
//! long does *this* machine take to execute the worst block the protocol lets it be sent?
//!
//! Two budgets bound a block independently, so both are measured:
//!
//! * **weight** — `max_block_validation_weight`, a size/IO estimate, enforced on receive. Measured by executing a real
//!   block's worth of canonical transfers and timing it.
//! * **execution points** — `max_block_validation_execution_points`, the actual metered cost, also enforced on receive.
//!   Measured as a marginal WASM rate, which is the figure the constants are expressed in.
//!
//! The transactions are built before the clock starts. Building signs them, and signing is the
//! wallet's cost; a validator verifies signatures at mempool admission, which
//! [`crate::native`] measures separately. What is timed here is exactly what the propose and
//! validate loops do — execute.
//!
//! State changes are deliberately not committed. Each execution therefore starts from the same
//! funded state and the sample is repeatable; committing would drain the account and make later
//! transactions in the run cheaper than earlier ones.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use tari_engine_types::commit_result::ExecuteResult;
use tari_ootle_transaction::{Transaction, args};
use tari_template_lib::types::{ComponentAddress, NonFungibleAddress, TemplateAddress, constants::TARI_TOKEN};
use tari_template_test_tooling::{Package, TemplateTest};

use crate::stats::Sample;

/// Fee ceiling stamped on every benchmark transaction. Only a bound — the actual fee is metered —
/// but it has to clear the most expensive shape built here (the wide `balance` transaction used for
/// the WASM slope), or that transaction traps out of allowance instead of executing.
const MAX_FEE: u64 = 60_000_000;

/// The compute-bound benchmark template, compiled to WASM by `build.rs` and embedded here so the
/// binary carries everything it executes. Its address is arbitrary but must not collide with a
/// builtin (those occupy 0x00..00, 0x00..01, 0x00..02 and 0x010203..).
const COMPUTE_BENCH_WASM: &[u8] = include_bytes!("../compiled/compute_bench.wasm");
const COMPUTE_BENCH_ADDRESS: TemplateAddress = TemplateAddress::from_array([0xBE; 32]);

/// Round counts the marginal WASM rate is fitted between.
///
/// The slope across these cancels every cost a call pays regardless of how much WASM it runs, so
/// what remains is execution alone. Both must stay well inside `MAX_WASM_POINTS_PER_TRANSACTION`
/// and inside what `MAX_FEE` funds, or the heavier call traps instead of finishing.
const WASM_SLOPE_ROUNDS: (u64, u64) = (5_000, 10_000);

/// Repeats of the whole-block execution. The minimum across repeats is what the rate is derived
/// from, so this only has to be large enough to have seen one uninterrupted pass.
const BLOCK_REPEATS: usize = 3;
const BLOCK_REPEATS_QUICK: usize = 1;

/// Trials per single-transaction timing. Cheap relative to a block pass, and the tail it exposes is
/// what the report's dispersion warning keys off.
const TX_TRIALS: usize = 25;
const TX_TRIALS_QUICK: usize = 7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionMeasurement {
    /// Wall time to compile the four builtin templates at startup. Not a consensus cost — a node
    /// pays it once, and caches compiled modules thereafter — but a clean single-threaded measure
    /// of this machine's compiler throughput, and the cost it pays again on every template publish.
    pub template_compile_ms: f64,
    /// The canonical block-filling transaction: withdraw from one account, deposit into another.
    pub transfer: TransferProfile,
    /// A full worst-case block, executed serially exactly as a replica would.
    pub block: BlockFill,
    /// Marginal metered points per millisecond of WASM execution, with fixed per-transaction
    /// overhead cancelled by the two-point slope. Directly comparable to the ~8.4M points/ms the
    /// block execution-point budgets are calibrated against.
    pub wasm_rate_points_per_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferProfile {
    /// `Transaction::calculate_transaction_weight` for this shape. The reference calibration used a
    /// ~62-weight transaction, so a materially different figure here means the shape has moved and
    /// the comparison to that calibration needs re-reading.
    pub weight: u64,
    /// WASM plus native points the execution actually charged.
    pub points: u64,
    pub time: Sample,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockFill {
    pub transactions: usize,
    pub total_weight: u64,
    pub total_points: u64,
    /// Best of [`BLOCK_REPEATS`] passes over the same block.
    pub elapsed_secs: f64,
    /// The headline figure, and the same one a running node logs from `on_propose` as its
    /// calibration signal — so a report can be checked against the node's own logs in production.
    pub weight_per_sec: f64,
    pub points_per_sec: f64,
}

/// Builds the harness and runs every execution measurement against it.
///
/// `target_weight` is the block size to fill, and should be the network's
/// `max_block_validation_weight`: the largest block a replica can be required to execute and vote
/// on within one view.
pub fn measure(target_weight: u64, quick: bool) -> anyhow::Result<ExecutionMeasurement> {
    // Compiling the builtin templates is the bulk of harness construction, and it is also a real
    // node cost — paid at startup and again on every template publish — so it is timed.
    let compile_started = Instant::now();
    let mut builder = Package::builder();
    builder.add_all_builtin_templates();
    builder
        .add_template_from_code(COMPUTE_BENCH_ADDRESS, COMPUTE_BENCH_WASM)
        .map_err(|e| anyhow::anyhow!("embedded compute_bench template failed to load: {e}"))?;
    let package = builder.build();
    let template_compile_ms = compile_started.elapsed().as_secs_f64() * 1000.0;

    let mut test = TemplateTest::from_package(package);
    test.bootstrap_state();

    // Fees on: a validator executes every transaction through the fee module, and the metering it
    // installs is part of the cost being measured.
    test.enable_fees();

    let (sender, sender_proof, sender_key) = test.create_funded_account();
    let (receiver, _, _) = test.create_empty_account();

    let tx_trials = if quick { TX_TRIALS_QUICK } else { TX_TRIALS };
    let block_repeats = if quick { BLOCK_REPEATS_QUICK } else { BLOCK_REPEATS };

    let transfer = measure_transfer(&mut test, sender, receiver, &sender_key, &sender_proof, tx_trials)?;
    let block = measure_block(
        &mut test,
        sender,
        receiver,
        &sender_key,
        &sender_proof,
        target_weight,
        block_repeats,
    )?;
    let wasm_rate_points_per_ms = measure_wasm_rate(&mut test, sender, &sender_key, &sender_proof, tx_trials)?;

    Ok(ExecutionMeasurement {
        template_compile_ms,
        transfer,
        block,
        wasm_rate_points_per_ms,
    })
}

/// The canonical block-filling transaction: a fee payment plus a one-token account-to-account
/// transfer. Everything in the weight budget is denominated in multiples of this shape.
///
/// Both accounts are declared as inputs, as a wallet would declare them — weight charges 15 per
/// input, so a transaction built without them is far lighter than the real thing and would let the
/// benchmark fit more executions into a block's weight than a validator ever has to. Any remaining
/// gap between this shape and production traffic errs the same safe way: under-declared weight
/// means more work per unit of weight, so the reported weight/s understates the machine rather than
/// flattering it. The weight actually used is reported so the shape can be checked against the
/// traffic being sized for.
fn build_transfer(
    test: &TemplateTest,
    sender: ComponentAddress,
    receiver: ComponentAddress,
    key: &tari_crypto::ristretto::RistrettoSecretKey,
) -> Transaction {
    test.transaction()
        .with_unversioned_inputs([sender, receiver])
        .pay_fee_from_component(sender, MAX_FEE)
        .call_method(sender, "withdraw", args![TARI_TOKEN, 1])
        .put_last_instruction_output_on_workspace("transferred")
        .call_method(receiver, "deposit", args![Workspace("transferred")])
        .build_and_seal(key)
}

fn measure_transfer(
    test: &mut TemplateTest,
    sender: ComponentAddress,
    receiver: ComponentAddress,
    key: &tari_crypto::ristretto::RistrettoSecretKey,
    proof: &NonFungibleAddress,
    trials: usize,
) -> anyhow::Result<TransferProfile> {
    let probe = build_transfer(test, sender, receiver, key);
    let weight = probe.calculate_transaction_weight().as_u64();

    // Warm up: the first execution of a template pays one-time lazy initialisation that no later
    // block ever pays again, and including it would understate the machine.
    let warmup = execute(test, probe, proof)?;
    let points = warmup.total_execution_points();

    let mut ms = Vec::with_capacity(trials);
    for _ in 0..trials {
        let transaction = build_transfer(test, sender, receiver, key);
        let started = Instant::now();
        let result = execute(test, transaction, proof)?;
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
        drop(result);
    }

    Ok(TransferProfile {
        weight,
        points,
        time: Sample::from_millis(ms),
    })
}

/// Fills a block to `target_weight` with canonical transfers and executes it serially, the way a
/// replica does when it votes. Reported from the fastest of `repeats` passes.
fn measure_block(
    test: &mut TemplateTest,
    sender: ComponentAddress,
    receiver: ComponentAddress,
    key: &tari_crypto::ristretto::RistrettoSecretKey,
    proof: &NonFungibleAddress,
    target_weight: u64,
    repeats: usize,
) -> anyhow::Result<BlockFill> {
    let mut total_weight = 0u64;
    let mut transactions = Vec::new();
    while total_weight < target_weight {
        let transaction = build_transfer(test, sender, receiver, key);
        total_weight += transaction.calculate_transaction_weight().as_u64();
        transactions.push(transaction);
    }

    let mut best_secs = f64::MAX;
    let mut total_points = 0u64;
    for _ in 0..repeats {
        // Rebuilding per pass keeps every pass identical in shape while giving each transaction a
        // distinct id, as it would have in a real block.
        let batch: Vec<Transaction> = (0..transactions.len())
            .map(|_| build_transfer(test, sender, receiver, key))
            .collect();

        let mut points = 0u64;
        let started = Instant::now();
        for transaction in batch {
            points = points.saturating_add(execute(test, transaction, proof)?.total_execution_points());
        }
        let secs = started.elapsed().as_secs_f64();
        if secs < best_secs {
            best_secs = secs;
            total_points = points;
        }
    }

    Ok(BlockFill {
        transactions: transactions.len(),
        total_weight,
        total_points,
        elapsed_secs: best_secs,
        weight_per_sec: total_weight as f64 / best_secs,
        points_per_sec: total_points as f64 / best_secs,
    })
}

/// Marginal WASM execution rate, in metered points per millisecond.
///
/// Fitted as a two-point slope between two calls of the embedded `ComputeBench::grind` that differ
/// only in their round count. The difference cancels everything a call pays regardless of how much
/// WASM it runs — transaction assembly, signature handling, fee intent, substate loads, invoke
/// setup — leaving the rate at which this machine turns metering points into wall-clock time.
///
/// A compute-bound template is required for this, not a convenience. Builtin methods wrap a small,
/// fixed amount of WASM in a comparatively large amount of engine work, and that work is charged
/// per instruction rather than per transaction, so no slope across instruction counts cancels it —
/// such a fit measures engine overhead and reports it as an execution rate, understating the
/// machine by more than an order of magnitude. The block execution-point budget is reachable only
/// by compute-bound transactions in the first place, so this is also the traffic the projection
/// has to be made against.
fn measure_wasm_rate(
    test: &mut TemplateTest,
    account: ComponentAddress,
    key: &tari_crypto::ristretto::RistrettoSecretKey,
    proof: &NonFungibleAddress,
    trials: usize,
) -> anyhow::Result<f64> {
    let (low, high) = WASM_SLOPE_ROUNDS;
    let (low_points, low_ms) = time_grind(test, account, key, proof, low, trials)?;
    let (high_points, high_ms) = time_grind(test, account, key, proof, high, trials)?;

    let point_slope = high_points.saturating_sub(low_points) as f64;
    let ms_slope = high_ms.min_ms - low_ms.min_ms;
    if ms_slope <= 0.0 || point_slope <= 0.0 {
        anyhow::bail!(
            "WASM rate slope is degenerate ({point_slope} points over {ms_slope:.4} ms); the machine is too noisy to \
             fit a rate, or the two probe sizes are too close together"
        );
    }
    Ok(point_slope / ms_slope)
}

fn time_grind(
    test: &mut TemplateTest,
    account: ComponentAddress,
    key: &tari_crypto::ristretto::RistrettoSecretKey,
    proof: &NonFungibleAddress,
    rounds: u64,
    trials: usize,
) -> anyhow::Result<(u64, Sample)> {
    let build = |test: &TemplateTest| {
        test.transaction()
            .pay_fee_from_component(account, MAX_FEE)
            .call_function(COMPUTE_BENCH_ADDRESS, "grind", args![rounds])
            .build_and_seal(key)
    };

    let warmup = execute(test, build(test), proof)?;
    // Points are deterministic for a given round count, so one observation fixes them; only the
    // wall clock needs repeating.
    let points = warmup.wasm_execution_points;

    let mut ms = Vec::with_capacity(trials);
    for _ in 0..trials {
        let transaction = build(test);
        let started = Instant::now();
        execute(test, transaction, proof)?;
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok((points, Sample::from_millis(ms)))
}

/// Executes without committing, and fails the run if the transaction did not succeed. A rejection
/// here is a bug in the benchmark's setup, not a property of the host, and its timing would be
/// meaningless — so it stops the run rather than quietly contributing a fast sample.
fn execute(
    test: &mut TemplateTest,
    transaction: Transaction,
    proof: &NonFungibleAddress,
) -> anyhow::Result<ExecuteResult> {
    let result = test.try_execute(transaction, vec![proof.clone()])?;
    if let Some(reason) = result.finalize.any_reject() {
        anyhow::bail!("benchmark transaction was rejected: {reason}");
    }
    Ok(result)
}
