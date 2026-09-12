//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Prices template instantiation in metering points.
//!
//! Every instruction that calls a template builds a fresh `Store` and `Instance`: linear memory is
//! mapped, the module's data segments are copied into it, and the tables and imports are wired up.
//! That work is proportional to the binary a publisher chose and runs before the first metered
//! operator, so it needs a price of its own.
//!
//! The measurement is a two-point fit over modules identical but for the size of their data
//! segment: the slope is the per-byte cost of the copy, the intercept everything fixed. Both are
//! converted to points at the same WASM points-per-millisecond rate
//! `native_points_calibrate` derives, and by the same method, so the figures are comparable.
//!
//! Run with `--release`; a debug build measures nothing useful.

use std::time::Instant;

use tari_engine::{fees::FeeTable, wasm::WasmModule};
use tari_engine_types::{fees::FeeSource, limits};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_test_tooling::TemplateTest;

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");
const METERING_BENCH: &str = "tests/templates/metering_bench";

/// Instantiations timed per data-segment size.
const TRIALS: usize = 200;
/// Cranelift compiles timed per template.
const COMPILE_TRIALS: usize = 5;
/// Executions timed per round count when deriving the WASM rate.
const ENGINE_TRIALS: usize = 7;
/// Round counts for the WASM rate fit.
const R1: u64 = 5_000;
const R2: u64 = 10_000;
const MAX_FEE: u64 = 60_000_000;

/// Data-segment sizes the fit runs over. The small one is about what a real template's rodata comes
/// to; the large one is near the publish size cap, where the copy dominates.
const SMALL_SEGMENT: usize = 4 * 1024;
const LARGE_SEGMENT: usize = 1024 * 1024;

/// See `tests/wasm_loader.rs` — the same encoded `TemplateDef` for a template named `Buggy` with a
/// single `main`.
const TEMPLATE_DEF: &[u8] = &[
    28, 0, 0, 0, 130, 0, 129, 131, 101, 66, 117, 103, 103, 121, 0, 129, 133, 100, 109, 97, 105, 110, 128, 130, 0, 128,
    244, 244,
];

fn wat_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

/// A minimal loadable template whose only variable is how many bytes its data segment carries.
fn module_with_data_segment(len: usize) -> Vec<u8> {
    let filler = wat_bytes(&vec![0x41u8; len]);
    let wat = format!(
        r#"
        (module
          (memory (export "memory") {pages})
          (data (i32.const 16) "\05\00\00\00\80")
          (data (i32.const 65536) "{filler}")
          (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "tari_free") (param i32))
          (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 20))
          (@custom "tari_tdef" "{def}")
        )
        "#,
        pages = limits::WASM_LIMITS.max_memory_pages,
        def = wat_bytes(TEMPLATE_DEF),
    );
    wat::parse_str(&wat).expect("hand-written module is valid wat")
}

/// Milliseconds one `Store` + `Instance` construction takes for a module with `segment_len` bytes
/// of data segment. Reports the minimum over `TRIALS`, which is the sample least polluted by the
/// scheduler.
fn instantiate_ms(segment_len: usize) -> (f64, usize) {
    let code = module_with_data_segment(segment_len);
    let code_size = code.len();
    let tari_engine::template::LoadedTemplate::Wasm(loaded) =
        WasmModule::load_template_from_code(&code).expect("module was rejected");

    let mut best = f64::MAX;
    for _ in 0..TRIALS {
        let start = Instant::now();
        let mut store = loaded.create_store();
        let imports = wasmer::imports! {
            "env" => {
                "tari_engine" => wasmer::Function::new_typed(&mut store, |_: i32, _: i32, _: i32| -> i32 { 0 }),
                "tari_debug" => wasmer::Function::new_typed(&mut store, |_: i32, _: i32| {}),
                "on_panic" => wasmer::Function::new_typed(&mut store, |_: i32, _: i32, _: i32, _: i32| {}),
            }
        };
        let instance = wasmer::Instance::new(&mut store, loaded.wasm_module(), &imports).expect("instantiation failed");
        let elapsed = start.elapsed().as_nanos() as f64 / 1e6;
        drop(instance);
        drop(store);
        best = best.min(elapsed);
    }

    (best, code_size)
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

/// Times instantiation of a real compiled template, so the price a `code_size`-based charge would
/// ask can be compared against what the template actually costs.
fn real_template_ms(path: &str) -> (f64, usize, u64, f64) {
    let code = std::fs::read(path).expect("template wasm");
    let code_size = code.len();
    let mut compile_ms = f64::MAX;
    for _ in 0..COMPILE_TRIALS {
        let start = Instant::now();
        WasmModule::load_template_from_code(&code).expect("module was rejected");
        compile_ms = compile_ms.min(start.elapsed().as_nanos() as f64 / 1e6);
    }
    let tari_engine::template::LoadedTemplate::Wasm(loaded) =
        WasmModule::load_template_from_code(&code).expect("module was rejected");
    let mut best = f64::MAX;
    for _ in 0..TRIALS {
        let start = Instant::now();
        let mut store = loaded.create_store();
        let imports = wasmer::imports! {
            "env" => {
                "tari_engine" => wasmer::Function::new_typed(&mut store, |_: i32, _: i32, _: i32| -> i32 { 0 }),
                "tari_debug" => wasmer::Function::new_typed(&mut store, |_: i32, _: i32| {}),
                "on_panic" => wasmer::Function::new_typed(&mut store, |_: i32, _: i32, _: i32, _: i32| {}),
            }
        };
        let instance = wasmer::Instance::new(&mut store, loaded.wasm_module(), &imports).expect("instantiation failed");
        let elapsed = start.elapsed().as_nanos() as f64 / 1e6;
        drop(instance);
        drop(store);
        best = best.min(elapsed);
    }
    (best, code_size, loaded.shape().data_segment_bytes, compile_ms)
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("WARNING: not a release build — timings are meaningless. Re-run with --release.");
    }

    let rate = wasm_rate_points_per_ms();
    println!("WASM rate: {rate:.0} points/ms");

    let (small_ms, small_size) = instantiate_ms(SMALL_SEGMENT);
    let (large_ms, large_size) = instantiate_ms(LARGE_SEGMENT);
    println!("  {small_size:>9} byte module: {small_ms:.4} ms");
    println!("  {large_size:>9} byte module: {large_ms:.4} ms");

    let per_byte_ms = (large_ms - small_ms) / (large_size - small_size) as f64;
    let fixed_ms = small_ms - per_byte_ms * small_size as f64;

    println!();
    println!(
        "fixed:    {fixed_ms:.4} ms -> {} points",
        (fixed_ms * rate).ceil() as u64
    );
    let per_byte_points = (per_byte_ms * rate).ceil() as u64;
    println!(
        "per byte: {:.6} ms/KiB -> {per_byte_points} points/byte",
        per_byte_ms * 1024.0,
    );

    println!();
    if let Ok(dir) = std::env::var("TEMPLATE_WASM_DIR") {
        for entry in std::fs::read_dir(dir).expect("template dir").flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "wasm") {
                continue;
            }
            let Some(path) = path.to_str() else { continue };
            let (ms, size, data, compile_ms) = real_template_ms(path);
            let measured = (ms * rate).ceil() as u64;
            let charged = limits::instantiation_points(data);
            println!(
                "{path}: {size} code / {data} data bytes\n  instantiate {ms:.4} ms = {measured} points measured, \
                 {charged} charged ({:.2}x)\n  compile {compile_ms:.3} ms = {} points, {:.1} points/code byte",
                charged as f64 / measured as f64,
                (compile_ms * rate).ceil() as u64,
                compile_ms * rate / size as f64,
            );
        }
    }
}
