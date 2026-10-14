//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Prices the host-side work of answering an engine call in metering points.
//!
//! Two figures are measured, both outside the Wasmer meter:
//!
//! - The response. Most handlers answer by encoding a value the engine holds, and the costliest is a component's state,
//!   a `Value` of up to `max_internal_call_size` bytes encoded afresh on every read. The engine then encodes the
//!   response, walks it for its item count and copies it into the template's memory. Each shape fills
//!   `max_internal_call_size` with the cheapest-to-encode item of its kind and times that whole sequence, so the worst
//!   of them bounds `PER_ENGINE_RESPONSE_ITEM`. The single byte string, which is nothing but copies, bounds
//!   `PER_ENGINE_RESPONSE_BYTE`.
//! - The flat cost of a call. A template loops reads of the current epoch, whose handler does next to nothing and whose
//!   response is a few items, so the native points it is charged are almost all `PER_ENGINE_CALL`. The time each extra
//!   call adds beyond its metered WASM is set against those points.
//!
//! Both are converted to points at the same WASM points-per-millisecond rate `native_points_calibrate`
//! derives, and by the same method, so the figures are comparable.
//!
//! Run with `--release`; a debug build measures nothing useful.

use std::{hint::black_box, time::Instant};

use tari_engine::fees::FeeTable;
use tari_engine_types::limits::{self, NativeExecutionPoints};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::args::InvokeResult;
use tari_template_test_tooling::TemplateTest;

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");
const METERING_BENCH: &str = "tests/templates/metering_bench";
const LIMITS: &str = "tests/templates/limits";

/// Times each shape is handled; the fastest is kept.
const TRIALS: usize = 50;
/// Executions timed per round count when deriving the WASM rate and the per-call cost.
const ENGINE_TRIALS: usize = 7;
/// Round counts for the WASM rate fit.
const R1: u64 = 5_000;
const R2: u64 = 10_000;
/// Engine call counts for the per-call fit.
const C1: u32 = 10_000;
const C2: u32 = 30_000;
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
fn value_shapes() -> Vec<(&'static str, tari_bor::Value)> {
    let mut map = head(5, (MAX - 5) / 2);
    map.extend(std::iter::repeat_n(0x00, (MAX - 5) / 2 * 2));
    let mut bytes = head(2, MAX - 5);
    bytes.extend(std::iter::repeat_n(0xab, MAX - 5));

    [
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
        ("h'abab..'", bytes),
    ]
    .into_iter()
    .map(|(label, raw)| (label, tari_bor::decode_exact(&raw).expect("decodes")))
    .collect()
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

/// A component's state read through an engine call: encoded into the handler's result, encoded as
/// the response, walked for its items and copied into the template's memory.
fn read_state(state: &tari_bor::Value, memory: &mut [u8]) -> u64 {
    let result = InvokeResult::encode(state).expect("encodes");
    let mut response = Vec::with_capacity(tari_bor::encoded_len(&result));
    tari_bor::encode_into_writer(&result, &mut response).expect("encodes");
    let items = tari_bor::count_data_items_bounded(&response, usize::MAX, u64::MAX)
        .expect("well-formed")
        .expect("within the item bound");
    memory[..response.len()].copy_from_slice(&response);
    items
}

/// A test whose fee table charges one fee unit per metered WASM point.
fn metered_test(templates: &[&str]) -> TemplateTest {
    let mut test = TemplateTest::new(CRATE_PATH, templates);
    let mut fee_table = FeeTable::zero_rated();
    fee_table.per_wasm_point_cost = 1;
    fee_table.wasm_points_cost_divisor = 1;
    test.set_fee_table(fee_table);
    test.enable_fees();
    test
}

/// The points-per-millisecond rate, derived exactly as `native_points_calibrate` derives it: two
/// round counts of the same metered loop, fitted on the marginal points over the marginal time.
fn wasm_rate_points_per_ms() -> f64 {
    let mut test = metered_test(&[METERING_BENCH]);
    let bench = test.get_template_address("MeteringBench");
    let (account, owner, key) = test.create_funded_account();

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

/// The time, metered WASM points and native points one extra current-epoch read adds, fitted over
/// two call counts as the WASM rate is.
fn per_engine_call() -> (f64, f64, f64) {
    // Fees stay off: the calls' native charges would outrun any fee this test could pay, and the
    // points are read from the execution itself.
    let mut test = TemplateTest::new(CRATE_PATH, [LIMITS]);
    let template = test.get_template_address("PushItToTheLimit");

    let mut run = |calls: u32| -> (u64, u64, f64) {
        let execute = |test: &mut TemplateTest| -> (u64, u64, f64) {
            let tx = Transaction::builder_localnet(Epoch(1))
                .call_function(template, "read_epoch_undecoded", args![calls])
                .build_and_seal(test.secret_key());
            let start = Instant::now();
            let result = test.execute_expect_success(tx, vec![]);
            let elapsed = start.elapsed().as_nanos() as f64 / 1e6;
            (result.wasm_execution_points, result.native_execution_points, elapsed)
        };
        let _ = execute(&mut test);
        let mut points = (0, 0);
        let mut ms = Vec::with_capacity(ENGINE_TRIALS);
        for _ in 0..ENGINE_TRIALS {
            let (wasm, native, t) = execute(&mut test);
            points = (wasm, native);
            ms.push(t);
        }
        ms.sort_by(f64::total_cmp);
        (points.0, points.1, ms[0])
    };

    let (wasm_c1, native_c1, ms_c1) = run(C1);
    let (wasm_c2, native_c2, ms_c2) = run(C2);
    let calls = f64::from(C2 - C1);
    (
        (ms_c2 - ms_c1) / calls,
        (wasm_c2 - wasm_c1) as f64 / calls,
        (native_c2 - native_c1) as f64 / calls,
    )
}

fn main() {
    let rate = wasm_rate_points_per_ms();
    println!("WASM rate: {rate:.0} points/ms\n");

    println!("Response: the handler's encode, the response encode, the item walk and the copy into memory");
    println!(
        "{:<24} {:>8} {:>8} {:>10} {:>12} {:>10}",
        "shape", "bytes", "items", "ms", "points/item", "charged/ms"
    );
    let mut memory = vec![0u8; 2 * MAX];
    for (label, state) in value_shapes() {
        let items = read_state(&state, &mut memory);
        let bytes = tari_bor::encode(&InvokeResult::encode(&state).unwrap()).unwrap().len();
        let ms = best_ms(|| read_state(&state, &mut memory));
        let points = ms * rate;
        let charged = limits::engine_response_points(bytes as u64, items);
        println!(
            "{label:<24} {bytes:>8} {items:>8} {ms:>10.3} {:>12.1} {:>9.2}x",
            points / items as f64,
            charged as f64 / points,
        );
    }

    let (ms, wasm, native) = per_engine_call();
    let host = ms * rate - wasm;
    println!("\nEngine call: one current-epoch read, its response left undecoded");
    println!(
        "{host:.0} points measured outside the {wasm:.0} metered WASM points, {native:.0} charged for them ({:.2}x), \
         {} of it the flat cost",
        native / host,
        NativeExecutionPoints::PER_ENGINE_CALL,
    );

    println!(
        "\nPER_ENGINE_RESPONSE_ITEM = {}, PER_ENGINE_RESPONSE_BYTE = {}, PER_ENGINE_CALL = {}. The charged columns \
         are what those charge over what was measured.",
        NativeExecutionPoints::PER_ENGINE_RESPONSE_ITEM,
        NativeExecutionPoints::PER_ENGINE_RESPONSE_BYTE,
        NativeExecutionPoints::PER_ENGINE_CALL,
    );
}
