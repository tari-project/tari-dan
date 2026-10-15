//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Prices moving non-fungible ids between containers in metering points.
//!
//! Taking from a bucket or vault, joining or depositing, and locking ids into a proof and unlocking
//! them again all move every id they carry, one at a time, on the host. Each round trip below moves
//! [`IDS`] ids out of a container and back, in each id shape, and the slowest shape bounds
//! `PER_NON_FUNGIBLE_ID_MOVED`. The ids are converted to points at the same WASM
//! points-per-millisecond rate `native_points_calibrate` derives, and by the same method, so the
//! figures are comparable.
//!
//! Run with `--release`; a debug build measures nothing useful.

use std::{collections::BTreeSet, hint::black_box, time::Instant};

use tari_engine::fees::FeeTable;
use tari_engine_types::{
    limits::{NativeExecutionPoints, non_fungible_move_points},
    resource_container::ResourceContainer,
};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::types::{NonFungibleId, constants::NFT_FAUCET_RESOURCE_ADDRESS};
use tari_template_test_tooling::TemplateTest;

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");
const METERING_BENCH: &str = "tests/templates/metering_bench";

/// Ids moved per round trip.
const IDS: usize = 50_000;
/// Times each round trip is run; the fastest is kept.
const TRIALS: usize = 20;
/// Executions timed per round count when deriving the WASM rate.
const ENGINE_TRIALS: usize = 7;
/// Round counts for the WASM rate fit.
const R1: u64 = 5_000;
const R2: u64 = 10_000;
const MAX_FEE: u64 = 60_000_000;

/// Ids of each shape. The strings are as long as an id may be and share all but their last few
/// characters, so every comparison reads them nearly to the end.
fn id_shapes() -> Vec<(&'static str, BTreeSet<NonFungibleId>)> {
    let prefix = "a".repeat(56);
    vec![
        ("u64", (0..IDS as u64).map(NonFungibleId::from_u64).collect()),
        (
            "u256",
            (0..IDS as u64)
                .map(|i| {
                    let mut id = [0u8; 32];
                    id[24..].copy_from_slice(&i.to_be_bytes());
                    NonFungibleId::from_u256(id)
                })
                .collect(),
        ),
        (
            "64-char string",
            (0..IDS)
                .map(|i| NonFungibleId::from_string(format!("{prefix}{i:08}")))
                .collect(),
        ),
    ]
}

/// The fastest of [`TRIALS`] runs of `f`, in milliseconds.
fn best_ms(mut f: impl FnMut()) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..TRIALS {
        let start = Instant::now();
        f();
        best = best.min(start.elapsed().as_nanos() as f64 / 1e6);
    }
    best
}

/// The points-per-millisecond rate, derived exactly as `native_points_calibrate` derives it: two
/// round counts of the same metered loop, fitted on the marginal points over the marginal time.
fn wasm_rate_points_per_ms() -> f64 {
    let mut test = TemplateTest::new(CRATE_PATH, [METERING_BENCH]);
    let bench = test.get_template_address("MeteringBench");
    let (account, owner, key) = test.create_funded_account();

    let mut fee_table = FeeTable::zero_rated();
    fee_table.per_wasm_point_cost = 1;
    fee_table.wasm_points_cost_divisor = 1;
    test.set_fee_table(fee_table);
    test.enable_fees();

    let mut run = |rounds: u64| -> (u64, f64) {
        let execute = |test: &mut TemplateTest| -> (u64, f64) {
            let tx = Transaction::builder_localnet(Epoch(1))
                .pay_fee_from_component(account, MAX_FEE)
                .call_function(bench, "bench_div_u64", args![rounds])
                .build_and_seal(&key);
            let start = Instant::now();
            let result = test.execute_expect_success(tx, vec![owner.clone()]);
            let elapsed = start.elapsed().as_nanos() as f64 / 1e6;
            (result.wasm_execution_points, elapsed)
        };
        let _ = execute(&mut test);
        let mut points = 0;
        let mut ms = Vec::with_capacity(ENGINE_TRIALS);
        for _ in 0..ENGINE_TRIALS {
            let (p, t) = execute(&mut test);
            points = p;
            ms.push(t);
        }
        ms.sort_by(f64::total_cmp);
        (points, ms[0])
    };

    let (points_r1, ms_r1) = run(R1);
    let (points_r2, ms_r2) = run(R2);
    (points_r2 - points_r1) as f64 / (ms_r2 - ms_r1)
}

fn main() {
    let rate = wasm_rate_points_per_ms();
    println!("WASM rate: {rate:.0} points/ms\n");
    println!(
        "{:<16} {:<22} {:>8} {:>10} {:>12} {:>10}",
        "shape", "round trip", "ids", "ms", "points/move", "charged/ms"
    );

    let address = NFT_FAUCET_RESOURCE_ADDRESS;
    let report = |shape: &str, round_trip: &str, ms: f64| {
        // Each round trip moves every id twice, and is charged for each move.
        let charged = 2 * non_fungible_move_points(IDS as u64);
        let points = ms * rate;
        println!(
            "{shape:<16} {round_trip:<22} {IDS:>8} {ms:>10.3} {:>12.1} {:>9.2}x",
            points / (2 * IDS) as f64,
            charged as f64 / points,
        );
    };

    for (shape, ids) in id_shapes() {
        let mut container = ResourceContainer::non_fungible(address, ids);
        report(
            shape,
            "take, then join",
            best_ms(|| {
                let taken = container.withdraw((IDS as u64).into()).unwrap();
                container.deposit(black_box(taken)).unwrap();
            }),
        );
        report(
            shape,
            "lock all, then unlock",
            best_ms(|| {
                let locked = container.lock_all().unwrap();
                container.unlock(black_box(locked)).unwrap();
            }),
        );
    }

    println!(
        "\nPER_NON_FUNGIBLE_ID_MOVED = {}. The last column is what that charges over what was measured.",
        NativeExecutionPoints::PER_NON_FUNGIBLE_ID_MOVED,
    );
}
