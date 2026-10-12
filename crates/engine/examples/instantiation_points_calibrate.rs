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

/// A module whose tables are filled by active element segments, which `Instance::new` writes out on
/// every instantiation just as it copies the data segments. `tables` x `entries` funcref writes.
fn module_with_element_segments(tables: usize, entries: usize) -> Vec<u8> {
    let funcrefs = vec!["0"; entries].join(" ");
    let table_decls = (0..tables)
        .map(|_| format!("(table {entries} {entries} funcref)"))
        .collect::<Vec<_>>()
        .join("\n");
    let elem_decls = (0..tables)
        .map(|i| format!("(elem (table {i}) (i32.const 0) func {funcrefs})"))
        .collect::<Vec<_>>()
        .join("\n");
    let wat = format!(
        r#"
        (module
          (memory (export "memory") {pages})
          (data (i32.const 16) "\05\00\00\00\80")
          {table_decls}
          (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "tari_free") (param i32))
          (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 20))
          {elem_decls}
          (@custom "tari_tdef" "{def}")
        )
        "#,
        pages = limits::WASM_LIMITS.max_memory_pages,
        def = wat_bytes(TEMPLATE_DEF),
    );
    wat::parse_str(&wat).expect("hand-written module is valid wat")
}

/// A module declaring tables but no element segments: the tables are still allocated and zeroed at
/// every instantiation, with nothing written into them.
fn module_with_tables_only(tables: usize, entries: usize) -> Vec<u8> {
    let table_decls = (0..tables)
        .map(|_| format!("(table {entries} {entries} funcref)"))
        .collect::<Vec<_>>()
        .join("\n");
    let wat = format!(
        r#"
        (module
          (memory (export "memory") {pages})
          (data (i32.const 16) "\05\00\00\00\80")
          {table_decls}
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

/// A module whose data section is `count` zero-length active segments. Each contributes nothing to
/// `data_segment_bytes`, but `Instance::new` still evaluates and bounds-checks every one.
fn module_with_empty_data_segments(count: usize) -> Vec<u8> {
    let segments = (0..count)
        .map(|_| "(data (i32.const 0) \"\")".to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let wat = format!(
        r#"
        (module
          (memory (export "memory") {pages})
          (data (i32.const 16) "\05\00\00\00\80")
          {segments}
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

/// The charge `code` attracts, read back from the shape its own loader derives.
fn charged_points(code: &[u8]) -> u64 {
    let tari_engine::template::LoadedTemplate::Wasm(loaded) =
        WasmModule::load_template_from_code(code).expect("module was rejected");
    limits::instantiation_points(&loaded.shape())
}

/// Milliseconds one instantiation of `code` takes, minimum over `TRIALS`.
fn instantiate_code_ms(code: &[u8]) -> f64 {
    let tari_engine::template::LoadedTemplate::Wasm(loaded) =
        WasmModule::load_template_from_code(code).expect("module was rejected");
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

/// A module of nothing but `count` empty functions, the most functions a binary of its size holds.
fn module_of_empty_functions(count: usize) -> Vec<u8> {
    let mut wat = String::from("(module (type (func))");
    wat.extend(std::iter::repeat_n("(func (type 0))", count));
    wat.push(')');
    wat::parse_str(wat).expect("valid wat")
}

/// A module of `functions` functions each declaring `locals` locals in one `(count, type)` run.
fn module_of_functions_with_locals(functions: usize, locals: usize) -> Vec<u8> {
    let func = format!("(func (local{}))", " i64".repeat(locals));
    let mut wat = String::from("(module");
    wat.extend(std::iter::repeat_n(func.as_str(), functions));
    wat.push(')');
    wat::parse_str(wat).expect("valid wat")
}

/// A module of `functions` functions whose body is `body`.
fn module_of_functions(functions: usize, body: &str) -> Vec<u8> {
    let func = format!("(func (local i32) {body})");
    let mut wat = String::from("(module (memory 1)");
    wat.extend(std::iter::repeat_n(func.as_str(), functions));
    wat.push(')');
    wat::parse_str(wat).expect("valid wat")
}

/// Best compile time of `code` over [`COMPILE_TRIALS`]. The module need not be a loadable template:
/// a refusal for a missing definition comes after the compile it times.
fn compile_ms(code: &[u8]) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..COMPILE_TRIALS {
        let start = Instant::now();
        let loaded = WasmModule::load_template_from_code(code);
        best = best.min(start.elapsed().as_nanos() as f64 / 1e6);
        drop(loaded);
    }
    best
}

/// Times instantiation of a real compiled template, so the price a `code_size`-based charge would
/// ask can be compared against what the template actually costs.
fn real_template_ms(path: &str) -> (f64, usize, u64, u64, f64) {
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
    (
        best,
        code_size,
        loaded.shape().data_segment_bytes,
        loaded.shape().element_segment_entries,
        compile_ms,
    )
}

/// Prints Cranelift's cost per function and per declared local, each expressed as the bytes the
/// byte price charges for the same time.
fn print_compile_costs(rate: f64) {
    // Cranelift's cost per function, expressed as the bytes the byte price charges for the same
    // time. `TEMPLATE_COMPILE_BYTES_PER_FUNCTION` must stay above it.
    let few = 1024;
    let many = limits::WASM_LIMITS.max_module_functions;
    let per_function_ms = (compile_ms(&module_of_empty_functions(many)) - compile_ms(&module_of_empty_functions(few))) /
        (many - few) as f64;
    println!(
        "per function: {:.1} us -> {} points, the byte price of {:.0} bytes (billed as {})",
        per_function_ms * 1000.0,
        (per_function_ms * rate).ceil() as u64,
        per_function_ms * rate / limits::PER_TEMPLATE_COMPILE_BYTE as f64,
        limits::TEMPLATE_COMPILE_BYTES_PER_FUNCTION,
    );

    // Cranelift's cost per declared local, expressed the same way.
    // `TEMPLATE_COMPILE_BYTES_PER_VARIABLE` must stay above it.
    let functions = 64;
    let locals = 8_000;
    let per_local_ms = (compile_ms(&module_of_functions_with_locals(functions, locals)) -
        compile_ms(&module_of_functions_with_locals(functions, 0))) /
        (functions * locals) as f64;
    println!(
        "per local: {:.1} ns -> {} points, the byte price of {:.2} bytes (billed as {})",
        per_local_ms * 1e6,
        (per_local_ms * rate).ceil() as u64,
        per_local_ms * rate / limits::PER_TEMPLATE_COMPILE_BYTE as f64,
        limits::TEMPLATE_COMPILE_BYTES_PER_VARIABLE,
    );

    print_metering_check_costs(rate);
    print_builtin_compile_counts(rate);
}

fn read_leb_u32(bytes: &[u8], pos: &mut usize) -> u32 {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = bytes[*pos];
        *pos += 1;
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            break;
        }
    }
    value
}

/// `code` without the custom sections a publish refuses, such as the `name` section the built-in
/// templates are compiled with.
fn strip_custom_sections(code: &[u8]) -> Vec<u8> {
    let mut stripped = code[..8].to_vec();
    let mut pos = 8;
    while pos < code.len() {
        let start = pos;
        let id = code[pos];
        pos += 1;
        let size = read_leb_u32(code, &mut pos) as usize;
        let end = pos + size;
        let keep = id != 0 || {
            let name_len = read_leb_u32(code, &mut pos) as usize;
            &code[pos..pos + name_len] == tari_template_abi::TEMPLATE_DEF_CUSTOM_SECTION.as_bytes()
        };
        if keep {
            stripped.extend_from_slice(&code[start..end]);
        }
        pos = end;
    }
    stripped
}

/// Prints what each built-in template's counts bill it as against its size, and its compile
/// against its charge. A real template should stay billed by its size.
fn print_builtin_compile_counts(rate: f64) {
    for template in tari_template_builtin::all_builtin_templates() {
        let code = &strip_custom_sections(template.binary);
        let structure = WasmModule::prevalidate_code(code).expect("built-in template is admitted");
        let counts = structure.compile;
        let charged_ms = limits::template_compile_points(code.len() as u64, &counts) as f64 / rate;
        let ms = compile_ms(code);
        println!(
            "{}: {} bytes, counts bill {} ({} checks, {} in the largest function); compile {ms:.1} ms, charged \
             {charged_ms:.1} ms ({:.2}x)",
            template.name,
            code.len(),
            limits::template_compile_count_bytes(&counts),
            counts.metering_checks,
            counts.largest_function_metering_checks,
            ms / charged_ms,
        );
    }
}

const MEMORY_FILL: &str = "(memory.fill (i32.const 0) (i32.const 0) (i32.const 0))";

/// Prints Cranelift's cost per metering check spread over many functions, expressed the same way,
/// its growth within one function, and the compile of the costliest single function the function
/// limits admit, against its charge.
fn print_metering_check_costs(rate: f64) {
    let max_checks = limits::WASM_LIMITS.max_function_metering_checks;
    let max_loops = limits::WASM_LIMITS.max_function_loops;
    let max_depth = limits::WASM_LIMITS.max_function_nesting_depth;

    // Modules asking for as much compile work as a module may, spread over a few large functions
    // or many small ones: wasmer's parallelism depends on how its functions bucket. A check follows
    // each `loop` and its `end`, and each `block`'s `end`, `br_if` and `memory.fill`.
    // `TEMPLATE_COMPILE_BYTES_PER_METERING_CHECK` is right when none of these compiles far above its
    // charge.
    let max_bytes = limits::ENGINE_LIMITS.max_template_binary_size_bytes as u64;
    for (name, body, checks_per_repeat) in [
        ("loops", "(loop)", 2),
        ("blocks", "(block)", 1),
        ("br_ifs", "(br_if 0 (local.get 0))", 1),
        ("memory.fills", MEMORY_FILL, 1),
    ] {
        for functions in [32u64, 128, 512, 2047] {
            // Less one check for the function's closing `end` and a byte for its local.
            let per_function = (max_bytes / functions - limits::TEMPLATE_COMPILE_BYTES_PER_FUNCTION - 1) /
                limits::TEMPLATE_COMPILE_BYTES_PER_METERING_CHECK -
                1;
            let loop_limit = (max_loops as u64).min(limits::WASM_LIMITS.max_module_loops as u64 / functions);
            let repeats = (per_function / checks_per_repeat)
                .min(max_checks as u64 / checks_per_repeat - 1)
                .min(if name == "loops" { loop_limit } else { u64::MAX });
            let code = module_of_functions(functions as usize, &body.repeat(repeats as usize));
            let Ok(structure) = WasmModule::prevalidate_code(&code) else {
                println!("{functions} functions of {repeats} {name}: refused");
                continue;
            };
            let charged_ms = limits::template_compile_points(code.len() as u64, &structure.compile) as f64 / rate;
            let ms = compile_ms(&code);
            println!(
                "{functions} functions of {repeats} {name} ({} bytes): {ms:.1} ms, charged {charged_ms:.1} ms ({:.1}x)",
                code.len(),
                ms / charged_ms
            );
        }
    }

    // A function's compile is `a·n + c·n²` in its checks `n`, so two sizes fit `c`.
    // `TEMPLATE_COMPILE_FUNCTION_CHECKS_SQUARED_PER_BYTE` must stay below the figure printed.
    for (name, body) in [("block", "(block)"), ("br_if", "(br_if 0 (local.get 0))")] {
        let n = max_checks / 2;
        let half = compile_ms(&module_of_functions(1, &body.repeat(n / 2)));
        let full = compile_ms(&module_of_functions(1, &body.repeat(n)));
        let c_ms = (full - 2.0 * half) / (n * n / 2) as f64;
        println!(
            "per {name} check squared in one function: {c_ms:.2e} ms -> one byte of the byte price per {:.0} (billed \
             per {})",
            limits::PER_TEMPLATE_COMPILE_BYTE as f64 / (c_ms * rate),
            limits::TEMPLATE_COMPILE_FUNCTION_CHECKS_SQUARED_PER_BYTE,
        );
    }

    // A function is compiled on one thread, so these are what the function limits bound.
    let loops_then_blocks = format!(
        "{}{}",
        "(loop)".repeat(max_loops),
        "(block)".repeat(max_checks - 2 * max_loops - 1)
    );
    let nested = format!(
        "{}{}{}",
        "(block ".repeat(max_depth - 1),
        "(block)".repeat(max_checks - max_depth),
        ")".repeat(max_depth - 1)
    );
    let worst_cases = [
        ("blocks", "(block)".repeat(max_checks - 1)),
        ("br_ifs", "(br_if 0 (local.get 0))".repeat(max_checks - 1)),
        ("memory.fills", MEMORY_FILL.repeat(max_checks - 1)),
        ("loops then blocks", loops_then_blocks),
        ("nested blocks", nested),
    ];
    for (name, body) in worst_cases {
        let code = module_of_functions(1, &body);
        let structure = WasmModule::prevalidate_code(&code).expect("within the function limits");
        let charged_ms = limits::template_compile_points(code.len() as u64, &structure.compile) as f64 / rate;
        let ms = compile_ms(&code);
        println!(
            "one function of {name} at the limits ({} bytes): {ms:.1} ms, charged {charged_ms:.1} ms ({:.1}x)",
            code.len(),
            ms / charged_ms
        );
    }
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

    // Element segments are the other thing copied per instantiation.
    let tables = limits::WASM_LIMITS.max_tables;
    let entries = limits::WASM_LIMITS.max_table_elements as usize;
    let none = instantiate_code_ms(&module_with_element_segments(0, 0));
    let full = instantiate_code_ms(&module_with_element_segments(tables, entries));
    println!();
    println!(
        "element segments: none {none:.4} ms, {tables}x{entries} {full:.4} ms -> {} points for {} writes",
        ((full - none) * rate).ceil() as i64,
        tables * entries,
    );

    // Declared table capacity, with nothing written into it.
    let tables_only = module_with_tables_only(tables, entries);
    let tables_only_ms = instantiate_code_ms(&tables_only);
    println!(
        "tables only ({tables}x{entries}, {} bytes): {tables_only_ms:.4} ms -> {} points, charged {}",
        tables_only.len(),
        ((tables_only_ms - none) * rate).ceil() as i64,
        charged_points(&tables_only),
    );

    // Zero-length active data segments.
    // wasmparser refuses a data section above 100,000 segments, so this is the whole range.
    for count in [1_000usize, 10_000] {
        let code = module_with_empty_data_segments(count);
        let ms = instantiate_code_ms(&code);
        println!(
            "{count} empty data segments ({} bytes): {ms:.4} ms -> {} points, charged {}",
            code.len(),
            ((ms - none) * rate).ceil() as i64,
            charged_points(&code),
        );
    }

    print_compile_costs(rate);

    println!();
    if let Ok(dir) = std::env::var("TEMPLATE_WASM_DIR") {
        for entry in std::fs::read_dir(dir).expect("template dir").flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "wasm") {
                continue;
            }
            let Some(path) = path.to_str() else { continue };
            let (ms, size, data, elements, compile_ms) = real_template_ms(path);
            let tari_engine::template::LoadedTemplate::Wasm(l) =
                WasmModule::load_template_from_code(&std::fs::read(path).unwrap()).unwrap();
            let artifact = l.wasm_module().serialize().unwrap().len();
            println!(
                "  artifact {artifact} bytes = {:.1}x the {size}-byte source",
                artifact as f64 / size as f64
            );
            let measured = (ms * rate).ceil() as u64;
            let charged = limits::instantiation_points(&l.shape());
            println!(
                "{path}: {size} code / {data} data bytes / {elements} elements\n  instantiate {ms:.4} ms = {measured} \
                 points measured, {charged} charged ({:.2}x)\n  compile {compile_ms:.3} ms = {} points, {:.1} \
                 points/code byte",
                charged as f64 / measured as f64,
                (compile_ms * rate).ceil() as u64,
                compile_ms * rate / size as f64,
            );
        }
    }
}
