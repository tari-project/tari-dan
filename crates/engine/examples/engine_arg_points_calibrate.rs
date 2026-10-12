//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Prices the host-side handling of an engine call's argument in metering points.
//!
//! A template hands the engine up to `max_internal_call_size` bytes of CBOR with every engine call.
//! The engine scans it for its item count and nesting depth, then decodes it into the op's argument
//! type. Neither runs under the Wasmer meter. The scan reads every item head before the item count
//! is known, so it is priced per byte at its densest, one item a byte. The decode is priced per item
//! once the scan has counted them.
//!
//! The scan is timed over arguments filling `max_internal_call_size` with the cheapest-to-encode
//! item of each kind. The decode is timed over the op arguments that hold the most items for their
//! size and decode in full: an invoke argument's `args` list of empty byte strings, and an event
//! payload of small entries. An argument that fails its decode stops early and costs less. Both are
//! converted to points at the same WASM points-per-millisecond rate `native_points_calibrate`
//! derives, and by the same method, so the figures are comparable.
//!
//! Run with `--release`; a debug build measures nothing useful.

use std::{hint::black_box, time::Instant};

use tari_engine::fees::FeeTable;
use tari_engine_types::{
    fees::FeeSource,
    limits::{self, NativeExecutionPoints},
};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::{
    args::{ComponentAction, ComponentInvokeArg, ComponentRef, EmitEventArg},
    types::{Metadata, bytes::Bytes},
};
use tari_template_test_tooling::TemplateTest;

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");
const METERING_BENCH: &str = "tests/templates/metering_bench";

/// Times each shape is handled; the fastest is kept.
const TRIALS: usize = 50;
/// Executions timed per round count when deriving the WASM rate.
const ENGINE_TRIALS: usize = 7;
/// Round counts for the WASM rate fit.
const R1: u64 = 5_000;
const R2: u64 = 10_000;
const MAX_FEE: u64 = 60_000_000;

const MAX: usize = limits::ENGINE_LIMITS.max_internal_call_size;

/// An array filling `max_internal_call_size` with copies of `item`.
fn array_of(item: &[u8]) -> Vec<u8> {
    let len = (MAX - 5) / item.len();
    let mut out = vec![0x9a];
    out.extend_from_slice(&u32::try_from(len).unwrap().to_be_bytes());
    out.extend(item.iter().copied().cycle().take(len * item.len()));
    out
}

fn scan_shapes() -> Vec<(&'static str, Vec<u8>)> {
    let mut chunked = vec![0x5f];
    chunked.extend(std::iter::repeat_n(0x40, MAX - 2));
    chunked.push(0xff);
    let mut bytes = vec![0x5a];
    bytes.extend_from_slice(&u32::try_from(MAX - 5).unwrap().to_be_bytes());
    bytes.extend(std::iter::repeat_n(0xab, MAX - 5));

    vec![
        ("[0, 0, ..]", array_of(&[0x00])),
        ("[null, null, ..]", array_of(&[0xf6])),
        ("[[], [], ..]", array_of(&[0x80])),
        ("[{}, {}, ..]", array_of(&[0xa0])),
        ("[[[0]], [[0]], ..]", array_of(&[0x81, 0x81, 0x00])),
        ("[0(0), 0(0), ..]", array_of(&[0xc0, 0x00])),
        ("(_ h'', h'', ..)", chunked),
        ("h'abab..'", bytes),
    ]
}

/// The largest `ComponentInvokeArg` whose `args` are all empty byte strings.
fn invoke_args() -> Vec<u8> {
    let arg = |n| ComponentInvokeArg {
        component_ref: ComponentRef::Component,
        action: ComponentAction::GetState,
        args: vec![Bytes::from(Vec::new()); n],
    };
    let overhead = tari_bor::encode(&arg(0)).unwrap().len() + 4;
    tari_bor::encode(&arg(MAX - overhead)).unwrap()
}

/// The largest `EmitEventArg` whose payload holds the shortest distinct keys, each mapped to `0`.
fn event_payload() -> Vec<u8> {
    let encode = |entries: u32| {
        let mut payload = Metadata::new();
        for i in 0..entries {
            payload.insert(format!("{i:x}"), &0u8);
        }
        tari_bor::encode(&EmitEventArg {
            topic: String::new(),
            payload,
        })
        .unwrap()
    };
    // Grows by many entries at a time, as re-encoding after every insert is quadratic.
    let mut entries = 0;
    while encode(entries + 1024).len() <= MAX {
        entries += 1024;
    }
    encode(entries)
}

/// The fastest of [`TRIALS`] runs of `f`, in milliseconds.
fn best_ms<T>(mut f: impl FnMut() -> T) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..TRIALS {
        let start = Instant::now();
        black_box(f());
        best = best.min(start.elapsed().as_nanos() as f64 / 1e6);
    }
    best
}

fn scan(raw: &[u8]) -> u64 {
    tari_bor::count_data_items_with_max_depth(raw, limits::MAX_CBOR_NESTING_DEPTH).expect("well-formed")
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
            let points = result
                .finalize
                .fee_receipt
                .fee_breakdown()
                .iter()
                .find_map(|(s, a)| (*s == FeeSource::WasmExecution).then_some(*a))
                .expect("WasmExecution charge present");
            (points, elapsed)
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

    println!("Scan, charged per byte before it runs");
    println!(
        "{:<22} {:>8} {:>8} {:>10} {:>12} {:>10}",
        "shape", "bytes", "items", "ms", "points/byte", "charged/ms"
    );
    for (label, raw) in scan_shapes() {
        let items = scan(&raw);
        let ms = best_ms(|| scan(&raw));
        let points = ms * rate;
        let charged = limits::engine_arg_scan_points(raw.len() as u64);
        println!(
            "{label:<22} {:>8} {items:>8} {ms:>10.3} {:>12.1} {:>9.2}x",
            raw.len(),
            points / raw.len() as f64,
            charged as f64 / points,
        );
    }

    println!("\nDecode, charged per item once the scan has counted them");
    println!(
        "{:<22} {:>8} {:>8} {:>10} {:>12} {:>10}",
        "shape", "bytes", "items", "ms", "points/item", "charged/ms"
    );
    let report = |label: &str, raw: &[u8], ms: f64| {
        let items = scan(raw);
        let points = ms * rate;
        let charged = limits::engine_arg_decode_points(items);
        println!(
            "{label:<22} {:>8} {items:>8} {ms:>10.3} {:>12.1} {:>9.2}x",
            raw.len(),
            points / items as f64,
            charged as f64 / points,
        );
    };
    let invoke = invoke_args();
    report(
        "invoke [h'', ..]",
        &invoke,
        best_ms(|| tari_bor::decode_exact::<ComponentInvokeArg>(&invoke).expect("decodes")),
    );
    let event = event_payload();
    report(
        "event {\"0\": 0, ..}",
        &event,
        best_ms(|| tari_bor::decode_exact::<EmitEventArg>(&event).expect("decodes")),
    );

    println!(
        "\nPER_ENGINE_ARG_BYTE = {}, PER_ENGINE_ARG_ITEM = {}. The last column is what those charge over what was \
         measured.",
        NativeExecutionPoints::PER_ENGINE_ARG_BYTE,
        NativeExecutionPoints::PER_ENGINE_ARG_ITEM,
    );
}
