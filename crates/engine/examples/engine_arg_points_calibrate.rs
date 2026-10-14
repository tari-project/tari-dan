//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Prices the host-side decode of CBOR a template wrote in metering points.
//!
//! A template hands the engine up to `max_internal_call_size` bytes of CBOR with every engine call,
//! and the engine decodes it several times over, all outside the Wasmer meter:
//!
//! - the engine call's argument, into the op's argument type;
//! - each byte string nested in that argument, into the handler's argument types. The costliest is a component's state,
//!   decoded as a `Value` that is indexed for well-known types before and after it replaces the old state, and compared
//!   with it;
//! - each argument of a cross-template call, nested a level further, decoded as a `Value` and encoded again into the
//!   callee's `CallInfo`.
//!
//! Each decode is charged by `engine_arg_points` after a walk that counts the items and checks
//! their nesting depth, so every figure below includes that walk. Each shape fills
//! `max_internal_call_size` with the cheapest-to-encode item of its kind, so the worst of them
//! bounds the per-item price. The per-byte price is the single byte string, which is nothing but
//! copies. Both are converted to points at the same WASM points-per-millisecond rate
//! `native_points_calibrate` derives, and by the same method, so the figures are comparable.
//!
//! Run with `--release`; a debug build measures nothing useful.

use std::{hint::black_box, time::Instant};

use tari_engine::fees::FeeTable;
use tari_engine_types::{
    fees::FeeSource,
    indexed_value::IndexedWellKnownTypes,
    limits::{self, NativeExecutionPoints},
};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_abi::CallInfo;
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

/// The CBOR head of a definite-length item of `major` type and `len`.
fn head(major: u8, len: usize) -> Vec<u8> {
    let mut out = vec![major << 5 | 26];
    out.extend_from_slice(&u32::try_from(len).unwrap().to_be_bytes());
    out
}

/// An array filling `max_internal_call_size` with copies of `item`.
fn array_of(item: &[u8]) -> Vec<u8> {
    let len = (MAX - 5) / item.len();
    let mut out = head(4, len);
    out.extend(item.iter().copied().cycle().take(len * item.len()));
    out
}

/// Values filling `max_internal_call_size`, one per item shape.
fn value_shapes() -> Vec<(&'static str, Vec<u8>)> {
    let mut map = head(5, (MAX - 5) / 2);
    map.extend(std::iter::repeat_n(0x00, (MAX - 5) / 2 * 2));
    let mut chunked = vec![0x5f];
    chunked.extend(std::iter::repeat_n(0x40, MAX - 2));
    chunked.push(0xff);
    let mut bytes = head(2, MAX - 5);
    bytes.extend(std::iter::repeat_n(0xab, MAX - 5));

    vec![
        ("[0, 0, ..]", array_of(&[0x00])),
        ("[null, null, ..]", array_of(&[0xf6])),
        ("[[], [], ..]", array_of(&[0x80])),
        ("[{}, {}, ..]", array_of(&[0xa0])),
        ("[[[0]], [[0]], ..]", array_of(&[0x81, 0x81, 0x00])),
        ("[\"\", \"\", ..]", array_of(&[0x60])),
        ("[\"a\", \"a\", ..]", array_of(&[0x61, 0x61])),
        ("[h'00', h'00', ..]", array_of(&[0x41, 0x00])),
        ("{0: 0, ..}", map),
        ("[0(0), 0(0), ..]", array_of(&[0xc0, 0x00])),
        ("[bucket(0), ..]", array_of(&[0xd8, 0x85, 0x00])),
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

/// The walk every charged decode runs first.
fn walk(raw: &[u8]) -> u64 {
    tari_bor::count_data_items_bounded(raw, limits::MAX_CBOR_NESTING_DEPTH, u64::MAX)
        .expect("well-formed")
        .expect("within the item bound")
}

/// A component's state replacing the old state: decoded, compared with the old, and indexed for
/// well-known types before and after.
fn set_state(raw: &[u8], old: &tari_bor::Value) -> bool {
    walk(raw);
    let state: tari_bor::Value = tari_bor::decode_exact(raw).expect("decodes");
    let before = IndexedWellKnownTypes::from_value(old);
    let after = IndexedWellKnownTypes::from_value(&state);
    black_box((before.is_ok(), after.is_ok()));
    state == *old
}

/// A cross-template call's argument: decoded and encoded into the callee's `CallInfo`.
fn call_arg(raw: &[u8]) -> Vec<u8> {
    walk(raw);
    let args = [tari_bor::decode_exact::<tari_bor::Value>(raw).expect("decodes")];
    let mut call_info = Vec::with_capacity(CallInfo::encode_v1_packed_size(&args).unwrap());
    CallInfo::encode_v1_packed(&mut call_info, 0, &args).unwrap();
    call_info
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
    let report = |label: &str, raw: &[u8], ms: f64| {
        let items = walk(raw);
        let points = ms * rate;
        let charged = limits::engine_arg_points(raw.len() as u64, items);
        println!(
            "{label:<24} {:>8} {items:>8} {ms:>10.3} {:>12.1} {:>9.2}x",
            raw.len(),
            points / items as f64,
            charged as f64 / points,
        );
    };
    let header = |title: &str| {
        println!("\n{title}");
        println!(
            "{:<24} {:>8} {:>8} {:>10} {:>12} {:>10}",
            "shape", "bytes", "items", "ms", "points/item", "charged/ms"
        );
    };

    header("Engine call argument: walk and decode into the op's argument type");
    let invoke = invoke_args();
    report(
        "invoke [h'', ..]",
        &invoke,
        best_ms(|| {
            walk(&invoke);
            tari_bor::decode_exact::<ComponentInvokeArg>(&invoke).expect("decodes")
        }),
    );
    let event = event_payload();
    report(
        "event {\"0\": 0, ..}",
        &event,
        best_ms(|| {
            walk(&event);
            tari_bor::decode_exact::<EmitEventArg>(&event).expect("decodes")
        }),
    );

    let shapes = value_shapes();
    header("Component state: walk, decode, compare, index before and after");
    for (label, raw) in &shapes {
        let old = tari_bor::decode_exact::<tari_bor::Value>(raw).expect("decodes");
        report(label, raw, best_ms(|| set_state(raw, &old)));
    }

    header("Cross-template call argument: walk, decode, encode into the callee's CallInfo");
    for (label, raw) in &shapes {
        report(label, raw, best_ms(|| call_arg(raw)));
    }

    println!(
        "\nPER_ENGINE_ARG_ITEM = {}, PER_ENGINE_ARG_BYTE = {}. The last column is what those charge over what was \
         measured.",
        NativeExecutionPoints::PER_ENGINE_ARG_ITEM,
        NativeExecutionPoints::PER_ENGINE_ARG_BYTE,
    );
}
