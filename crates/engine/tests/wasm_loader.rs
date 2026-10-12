//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Admission rules the engine applies to an untrusted template binary, and the bounds it keeps over
//! one while it runs. Every module here is hand-written: the `#[template]` macro cannot express a
//! module that declares a start function, an oversized table, or a malformed ABI section, and those
//! are exactly the shapes a published binary may arrive in.

use std::cell::Cell;

use tari_engine::wasm::{WasmExecutionError, WasmModule, WasmValidationError};
use tari_engine_types::{
    commit_result::{ExecutionFailureCode, RejectReason, TransactionResult},
    hashing::hash_template_code,
    limits,
};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::types::TemplateAddress;
use tari_template_test_tooling::{Package, TemplateTest};

/// Bor-encoded `TemplateDef::V1(TemplateDefV1 { template_name: "Buggy", abi_version: 0, functions:
/// [FunctionDef { name: "main", arguments: [], output: Type::Unit, is_mut: false, is_migration:
/// false }] })`, behind the 4-byte little-endian length prefix `encode_for_wasm_embedding` adds.
/// Shared with `tests/templates/buggy`, which embeds the same blob.
const TEMPLATE_DEF: &[u8] = &[
    28, 0, 0, 0, 130, 0, 129, 131, 101, 66, 117, 103, 103, 121, 0, 129, 133, 100, 109, 97, 105, 110, 128, 130, 0, 128,
    244, 244,
];

/// Renders bytes as a WAT string literal.
fn wat_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

/// A module carrying everything the loader requires of a template — the ABI section, the memory,
/// the entrypoint and the allocator pair — with `parts` splicing in whatever the test is about.
///
/// `tari_alloc` hands out one fixed region, at offset 1024: the engine stages a single `CallInfo`
/// per call, and nothing here allocates again. `Buggy_main` returns a pointer to the
/// `[u32 alloc_len][payload]` pair at offset 16 — clear of that region — whose payload is the
/// encoded unit the declared return type expects.
fn template_module(parts: &str) -> Vec<u8> {
    template_module_importing("", parts)
}

/// [`template_module`] for a module that also imports host functions, which the text format
/// requires to come before any definition.
fn template_module_importing(imports: &str, parts: &str) -> Vec<u8> {
    let wat = format!(
        r#"
        (module
          {imports}
          (memory (export "memory") 3)
          (data (i32.const 16) "\05\00\00\00\80")
          {parts}
          (@custom "tari_tdef" "{}")
        )
        "#,
        wat_bytes(TEMPLATE_DEF)
    );
    wat::parse_str(&wat).unwrap()
}

/// The allocator pair and entrypoint a template that only has to load needs.
const ABI_EXPORTS: &str = r#"
    (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
    (func (export "tari_free") (param i32))
    (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 20))
"#;

fn validation_error(code: &[u8]) -> String {
    WasmModule::validate_code(code)
        .expect_err("module was accepted")
        .to_string()
}

#[test]
fn accepts_a_hand_written_template() {
    let def = WasmModule::validate_code(&template_module(ABI_EXPORTS)).unwrap();
    assert_eq!(def.template_name(), "Buggy");
}

#[test]
fn rejects_a_start_section() {
    // The start function is referenced by index rather than by name: a named symbol makes `wat`
    // emit a `name` custom section, which the loader rejects before it looks at anything else.
    let code = template_module(&format!(
        r#"
        {ABI_EXPORTS}
        (func)
        (start 3)
        "#
    ));

    let err = WasmModule::validate_code(&code).expect_err("module with a start section was accepted");
    assert!(
        matches!(
            err,
            tari_engine::template::TemplateLoaderError::WasmModuleError(WasmExecutionError::WasmValidationError(
                WasmValidationError::StartSectionNotAllowed
            ))
        ),
        "unexpected error: {err}"
    );
}

#[test]
fn rejects_a_table_declaring_more_elements_than_the_limit() {
    let over_limit = limits::WASM_LIMITS.max_table_elements + 1;

    // A declared maximum above the limit.
    let err = validation_error(&template_module(&format!(
        r#"
        {ABI_EXPORTS}
        (table 1 {over_limit} funcref)
        "#
    )));
    assert!(err.contains("table element limit"), "unexpected error: {err}");

    // A minimum above the limit, which the host allocates outright at instantiation.
    let err = validation_error(&template_module(&format!(
        r#"
        {ABI_EXPORTS}
        (table {over_limit} funcref)
        "#
    )));
    assert!(err.contains("table element limit"), "unexpected error: {err}");
}

#[test]
fn rejects_a_memory_declaring_more_pages_than_the_limit() {
    let over_limit = limits::WASM_LIMITS.max_memory_pages + 1;
    let code = wat::parse_str(format!(
        r#"
        (module
          (memory (export "memory") {over_limit})
          {ABI_EXPORTS}
          (@custom "tari_tdef" "{}")
        )
        "#,
        wat_bytes(TEMPLATE_DEF)
    ))
    .unwrap();

    let err = validation_error(&code);
    assert!(err.contains("memory limit"), "unexpected error: {err}");
}

#[test]
fn rejects_a_module_without_a_template_def_section() {
    let code = wat::parse_str(format!(
        r#"
        (module
          (memory (export "memory") 1)
          {ABI_EXPORTS}
        )
        "#
    ))
    .unwrap();

    let err = validation_error(&code);
    assert!(err.contains("tari_tdef"), "unexpected error: {err}");
}

/// The legacy embedding: a guest-controlled `_ABI_TEMPLATE_DEF` global pointing into linear memory.
/// The engine reads the ABI from the module's own section and never from guest memory, so a module
/// that carries only the global is one without an ABI.
#[test]
fn rejects_a_module_carrying_only_the_legacy_abi_global() {
    let code = wat::parse_str(
        r#"
        (module
          (memory (export "memory") 1)
          (global (export "_ABI_TEMPLATE_DEF") i32 (i32.const -1))
        )
        "#,
    )
    .unwrap();

    let err = validation_error(&code);
    assert!(err.contains("tari_tdef"), "unexpected error: {err}");
}

#[test]
fn rejects_a_malformed_template_def_section() {
    // Shorter than the length prefix.
    let code = wat::parse_str(format!(
        r#"
        (module
          (memory (export "memory") 1)
          {ABI_EXPORTS}
          (@custom "tari_tdef" "\01\02")
        )
        "#
    ))
    .unwrap();
    let err = validation_error(&code);
    assert!(err.contains("length prefix"), "unexpected error: {err}");

    // A length prefix that overruns the section.
    let code = wat::parse_str(format!(
        r#"
        (module
          (memory (export "memory") 1)
          {ABI_EXPORTS}
          (@custom "tari_tdef" "\ff\00\00\00\80")
        )
        "#
    ))
    .unwrap();
    let err = validation_error(&code);
    assert!(err.contains("inconsistent"), "unexpected error: {err}");

    // A well-formed prefix over a payload that is not a `TemplateDef`.
    let code = wat::parse_str(format!(
        r#"
        (module
          (memory (export "memory") 1)
          {ABI_EXPORTS}
          (@custom "tari_tdef" "\05\00\00\00\ff")
        )
        "#
    ))
    .unwrap();
    let err = validation_error(&code);
    assert!(err.contains("decode template definition"), "unexpected error: {err}");
}

#[test]
fn rejects_more_tables_than_the_limit() {
    let tables = "(table 1 funcref)\n".repeat(limits::WASM_LIMITS.max_tables + 1);
    let code = template_module(&format!(
        r#"
        {ABI_EXPORTS}
        {tables}
        "#
    ));

    let err = validation_error(&code);
    assert!(err.contains("tables"), "unexpected error: {err}");
}

#[test]
fn rejects_more_globals_than_the_limit() {
    let globals = "(global i32 (i32.const 0))\n".repeat(limits::WASM_LIMITS.max_globals + 1);
    let code = template_module(&format!(
        r#"
        {ABI_EXPORTS}
        {globals}
        "#
    ));

    let err = validation_error(&code);
    assert!(err.contains("globals"), "unexpected error: {err}");
}

/// The engine calls `tari_alloc` and `tari_free` on every invocation, so a module that exports
/// neither — or exports them under another signature — is refused at admission.
#[test]
fn rejects_a_missing_or_mistyped_allocator() {
    let code = wat::parse_str(format!(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 20))
          (@custom "tari_tdef" "{}")
        )
        "#,
        wat_bytes(TEMPLATE_DEF)
    ))
    .unwrap();
    let err = validation_error(&code);
    assert!(err.contains("tari_alloc"), "unexpected error: {err}");

    let code = template_module(
        r#"
        (func (export "tari_alloc") (param i64) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 20))
        "#,
    );
    let err = validation_error(&code);
    assert!(err.contains("tari_alloc"), "unexpected error: {err}");
}

#[test]
fn rejects_an_entrypoint_with_the_wrong_signature() {
    let code = template_module(
        r#"
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32) (result i32) (i32.const 20))
        "#,
    );

    let err = validation_error(&code);
    assert!(err.contains("Buggy_main"), "unexpected error: {err}");
}

#[test]
fn rejects_a_module_without_a_memory_export() {
    let code = wat::parse_str(format!(
        r#"
        (module
          (memory 1)
          {ABI_EXPORTS}
          (@custom "tari_tdef" "{}")
        )
        "#,
        wat_bytes(TEMPLATE_DEF)
    ))
    .unwrap();

    let err = validation_error(&code);
    assert!(err.contains("`memory`"), "unexpected error: {err}");
}

#[test]
fn rejects_an_unexpected_exported_function() {
    let code = template_module(&format!(
        r#"
        {ABI_EXPORTS}
        (func (export "i_shouldnt_be_here") (result i32) (i32.const 0))
        "#
    ));

    let err = validation_error(&code);
    assert!(err.contains("i_shouldnt_be_here"), "unexpected error: {err}");
}

/// Loads `code` as a template and calls its `main`, returning the WASM points the call consumed.
///
/// The harness's template provider is a fixed map, so the module is registered directly rather than
/// published: these tests are about what the engine does with a template while it runs, not about
/// the publishing path.
fn load_and_call(code: Vec<u8>) -> Result<u64, RejectReason> {
    let (mut test, address) = load(code);
    call_main(&mut test, address)
}

fn load(code: Vec<u8>) -> (TemplateTest, TemplateAddress) {
    let address: TemplateAddress = hash_template_code(&code);
    let mut builder = Package::builder();
    builder.add_all_builtin_templates();
    builder
        .add_template_from_code(address, code)
        .expect("template was rejected by the loader");
    let mut test = TemplateTest::from_package(builder.build());
    test.bootstrap_state();
    (test, address)
}

fn call_main(test: &mut TemplateTest, address: TemplateAddress) -> Result<u64, RejectReason> {
    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .call_function(address, "main", args![])
                .build_and_seal(test.secret_key()),
            vec![],
        )
        .expect("execution failed");

    match result.finalize.result {
        TransactionResult::Accept(_) => Ok(result.wasm_execution_points),
        TransactionResult::Reject(reason) | TransactionResult::AcceptFeeRejectRest(_, reason) => Err(reason),
    }
}

/// A table is immutable once instantiated, so a reused instance never needs its tables restored:
/// `table.grow` is refused when the module is loaded.
#[test]
fn a_table_growing_module_is_refused() {
    let code = template_module(
        r#"
        (table 1 funcref)
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32)
          (drop (table.grow 0 (ref.null func) (i32.const 1)))
          (i32.const 20))
        "#,
    );

    let err = WasmModule::load_template_from_code(&code).expect_err("a table-growing module was accepted");
    assert!(err.to_string().contains("modifies a table"), "unexpected error: {err}");
}

/// The value a template returns is bounded like the arguments passed into it. Without a bound the
/// engine decodes, validates and carries whatever a template writes into its linear memory.
#[test]
fn an_oversized_return_value_is_rejected() {
    let payload_len = limits::ENGINE_LIMITS.max_call_size + 1;
    let alloc_len = (payload_len + 4) as u32;
    // The blob sits past the region `tari_alloc` hands out, which holds this call's `CallInfo`.
    let code = template_module(&format!(
        r#"
        (data (i32.const 1048) "{}")
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 1052))
        "#,
        wat_bytes(&alloc_len.to_le_bytes())
    ));

    let reason = load_and_call(code).expect_err("an oversized return value was accepted");
    let RejectReason::ExecutionFailure { code, message } = reason else {
        panic!("expected an execution failure, got {reason:?}");
    };
    assert_eq!(code, ExecutionFailureCode::LimitExceeded);
    assert!(
        message.contains(&limits::ENGINE_LIMITS.max_call_size.to_string()),
        "unexpected error: {message}"
    );
}

/// `tari_alloc` is template code the engine drives, so what it spends is charged to the transaction
/// like the template function's own consumption.
#[test]
fn points_spent_in_tari_alloc_are_charged() {
    let code = template_module(
        r#"
        (func (export "tari_alloc") (param i32) (result i32)
          (local $i i32)
          (local.set $i (i32.const 100000))
          (block $done
            (loop $again
              (br_if $done (i32.eqz (local.get $i)))
              (local.set $i (i32.sub (local.get $i) (i32.const 1)))
              (br $again)))
          (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 20))
        "#,
    );

    let points = load_and_call(code).expect("call failed");
    // The entrypoint itself costs a handful of points, so anything on this scale can only have come
    // from the allocator's loop.
    assert!(points > 100_000, "only {points} points were charged");
}

/// `memory.copy` is one operator whose work is a runtime operand, so its charge must follow the
/// length it is given rather than the flat cost of the instruction.
#[test]
fn memory_copy_points_scale_with_the_bytes_copied() {
    fn points_for_copy_of(len: u32) -> u64 {
        let code = template_module(&format!(
            r#"
            (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
            (func (export "tari_free") (param i32))
            (func (export "Buggy_main") (param i32 i32) (result i32)
              (memory.copy (i32.const 65536) (i32.const 0) (i32.const {len}))
              (i32.const 20))
            "#
        ));
        load_and_call(code).expect("call failed")
    }

    let empty = points_for_copy_of(0);
    let one_page = points_for_copy_of(65_536);

    assert_eq!(
        one_page - empty,
        65_536,
        "a 64 KiB copy cost {one_page} points against {empty} for an empty one"
    );
}

/// A copy small enough to be what a template actually does stays close to the flat cost, so the
/// length-proportional charge does not price ordinary code out of the budget.
#[test]
fn a_small_memory_copy_stays_cheap() {
    let code = template_module(
        r#"
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32)
          (memory.copy (i32.const 65536) (i32.const 0) (i32.const 32))
          (i32.const 20))
        "#,
    );

    let points = load_and_call(code).expect("call failed");
    assert!(points < 1_000, "a 32-byte copy cost {points} points");
}

/// The length charge is emitted before the copy runs, so a length no budget can cover fails on the
/// meter rather than on wasmer's bounds check — the meter is what bounds the CPU a transaction may
/// claim, and a bounds trap would mean the copy was attempted first.
#[test]
fn an_unbounded_memory_copy_traps_on_the_meter() {
    let code = template_module(
        r#"
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32)
          (memory.copy (i32.const 0) (i32.const 0) (i32.const -1))
          (i32.const 20))
        "#,
    );

    let reason = load_and_call(code).expect_err("an unbounded copy was accepted");
    // The meter must be what stops it. A bounds check would refuse the copy for free, leaving a
    // module able to ask for unbounded work and pay for none of it.
    assert_eq!(
        reason.execution_failure_code(),
        Some(ExecutionFailureCode::LimitExceeded),
        "the copy did not trap on the meter: {reason}"
    );
    assert!(
        !reason.to_string().contains("out of bounds"),
        "the copy did not trap on the meter: {reason}"
    );
}

/// `memory.grow` past what the tunables can grant returns -1 having moved nothing, so the charge is
/// taken on the pages that could actually be granted. Billing the requested delta would price a
/// refusal like a success, and a large enough request would exhaust the meter and trap where the
/// module is entitled to its -1.
#[test]
fn an_impossible_memory_grow_is_charged_only_for_what_could_be_granted() {
    fn points_for_grow_of(pages: i64) -> Result<u64, RejectReason> {
        load_and_call(template_module(&format!(
            r#"
            (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
            (func (export "tari_free") (param i32))
            (func (export "Buggy_main") (param i32 i32) (result i32)
              (if (i32.ne (memory.grow (i32.const {pages})) (i32.const -1))
                (then unreachable))
              (i32.const 20))
            "#
        )))
    }

    // Both are refused by the tunables — the module declares 3 of the 32 pages it may have — so both
    // do the same zero work and must cost the same.
    let just_over = points_for_grow_of(limits::WASM_LIMITS.max_memory_pages as i64).expect("call failed");
    let absurd = points_for_grow_of(1_000_000).expect("call failed");

    assert_eq!(
        absurd, just_over,
        "a 1,000,000-page request cost {absurd} against {just_over} for one just over the cap"
    );
}

/// The charge sequence `BulkMetering` emits is invisible to the static cost function by
/// construction, so the static cost of the operators it instruments has to cover it. Otherwise a
/// module repeating a zero-length copy executes that sequence for free.
#[test]
fn a_zero_length_copy_pays_for_the_charge_sequence() {
    const COPIES: u64 = 100;

    fn points_for_copies(n: u64) -> u64 {
        let body = (0..n)
            .map(|_| "(memory.copy (i32.const 65536) (i32.const 0) (i32.const 0))")
            .collect::<Vec<_>>()
            .join("\n");
        load_and_call(template_module(&format!(
            r#"
            (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
            (func (export "tari_free") (param i32))
            (func (export "Buggy_main") (param i32 i32) (result i32)
              {body}
              (i32.const 20))
            "#
        )))
        .expect("call failed")
    }

    // Three `i32.const` operands at 1 point each accompany every copy.
    const OPERAND_COST: u64 = 3;
    let marginal = (points_for_copies(COPIES) - points_for_copies(0)) / COPIES - OPERAND_COST;

    // The executed sequence is ~12 points; the charge must at least cover it.
    assert!(
        marginal >= 12,
        "a zero-length copy was charged {marginal} points, less than the sequence it runs"
    );
}

/// Element segments are written into the instance's tables at every instantiation, just as data
/// segments are copied into its memory, and the module author chooses how many entries there are.
/// Charging only the flat instantiation cost would let a table-heavy template buy that work for
/// nothing.
#[test]
fn element_segment_entries_are_charged_per_instantiation() {
    use tari_engine_types::limits::{PER_TEMPLATE_ELEMENT_ENTRY, instantiation_points};

    const ENTRIES: u64 = 4096;

    let funcrefs = vec!["0"; ENTRIES as usize].join(" ");
    let code = template_module(&format!(
        r#"
        (table {ENTRIES} {ENTRIES} funcref)
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 20))
        (elem (i32.const 0) func {funcrefs})
        "#
    ));

    let shape = match WasmModule::load_template_from_code(&code).expect("module was rejected") {
        tari_engine::template::LoadedTemplate::Wasm(loaded) => loaded.shape(),
    };

    assert_eq!(shape.element_segment_entries, ENTRIES);
    assert!(
        instantiation_points(&shape) >= ENTRIES * PER_TEMPLATE_ELEMENT_ENTRY,
        "a {ENTRIES}-entry table was not charged for its entries"
    );
}

/// Every segment must be dropped at instantiation, so that `memory.init`, `table.init` and the
/// drop operators can change nothing a reused instance would need restored: a passive segment is
/// refused when the module is loaded.
#[test]
fn passive_segments_are_refused() {
    for segment in [r#"(data "passive bytes")"#, "(elem func 0)"] {
        let code = template_module(&format!(
            r#"
            (table 4 4 funcref)
            {segment}
            (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
            (func (export "tari_free") (param i32))
            (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 20))
            "#
        ));

        let err = WasmModule::load_template_from_code(&code).expect_err("a passive segment was accepted");
        assert!(
            err.to_string().contains("passive"),
            "unexpected error for {segment}: {err}"
        );
    }
}

/// Nothing caps how many element segments a module declares, and several may target one table at
/// overlapping offsets — each is written out in turn at instantiation. The entry count a charge is
/// taken on is therefore bounded by the binary, not by the table limits.
#[test]
fn overlapping_element_segments_each_count() {
    const SEGMENTS: u64 = 8;
    const ENTRIES: u64 = 512;

    let funcrefs = vec!["0"; ENTRIES as usize].join(" ");
    let segments = (0..SEGMENTS)
        .map(|_| format!("(elem (i32.const 0) func {funcrefs})"))
        .collect::<Vec<_>>()
        .join("\n");
    let code = template_module(&format!(
        r#"
        (table {ENTRIES} {ENTRIES} funcref)
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 20))
        {segments}
        "#
    ));

    let shape = match WasmModule::load_template_from_code(&code).expect("module was rejected") {
        tari_engine::template::LoadedTemplate::Wasm(loaded) => loaded.shape(),
    };

    // Every segment is counted, even though the table only ever holds `ENTRIES` of them at once.
    assert_eq!(shape.element_segment_entries, SEGMENTS * ENTRIES);
}

/// A zero-length active data segment contributes no bytes but is still walked, offset-evaluated and
/// bounds-checked at every instantiation. Pricing data segments by payload alone would make a
/// section full of them free, and nothing caps how many a module declares.
#[test]
fn empty_data_segments_are_charged_per_segment() {
    use tari_engine_types::limits::{PER_TEMPLATE_DATA_SEGMENT, instantiation_points};

    const SEGMENTS: u64 = 64;

    let empties = (0..SEGMENTS)
        .map(|_| r#"(data (i32.const 0) "")"#)
        .collect::<Vec<_>>()
        .join("\n");
    let code = template_module(&format!(
        r#"
        {empties}
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 20))
        "#
    ));

    let shape = match WasmModule::load_template_from_code(&code).expect("module was rejected") {
        tari_engine::template::LoadedTemplate::Wasm(loaded) => loaded.shape(),
    };

    // `template_module` carries one active segment of its own.
    assert_eq!(shape.data_segment_count, SEGMENTS + 1);
    assert!(
        instantiation_points(&shape) >= SEGMENTS * PER_TEMPLATE_DATA_SEGMENT,
        "{SEGMENTS} empty segments were not charged for"
    );
}

/// Tables are allocated and zeroed at every instantiation whether or not an element segment writes
/// to them, and a module claims that storage in a handful of bytes.
#[test]
fn declared_table_capacity_is_charged_without_any_element_segment() {
    use tari_engine_types::limits::{PER_TEMPLATE_TABLE_SLOT, instantiation_points};

    const SLOTS: u64 = 4096;

    let code = template_module(&format!(
        r#"
        (table {SLOTS} {SLOTS} funcref)
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 20))
        "#
    ));

    let shape = match WasmModule::load_template_from_code(&code).expect("module was rejected") {
        tari_engine::template::LoadedTemplate::Wasm(loaded) => loaded.shape(),
    };

    assert_eq!(shape.declared_table_slots, SLOTS);
    assert_eq!(shape.element_segment_entries, 0);
    assert!(
        instantiation_points(&shape) >= SLOTS * PER_TEMPLATE_TABLE_SLOT,
        "a {SLOTS}-slot table was not charged for"
    );
}

/// What the engine wrote to the validator's log while one call ran.
#[derive(Debug, Clone, Copy, Default)]
struct Logged {
    records: usize,
    bytes: usize,
}

thread_local! {
    static LOGGED: Cell<Logged> = const { Cell::new(Logged { records: 0, bytes: 0 }) };
}

/// Counts records at the levels a validator logs by default. The engine runs a call on the thread
/// that submits it, so a thread-local tally sees only the test's own call.
struct CountingLogger;

impl log::Log for CountingLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let bytes = record.args().to_string().len();
        LOGGED.with(|logged| {
            let Logged { records, bytes: total } = logged.get();
            logged.set(Logged {
                records: records + 1,
                bytes: total + bytes,
            });
        });
    }

    fn flush(&self) {}
}

/// Loads `code` and calls its `main`, tallying what the call alone logs.
fn call_logged(code: Vec<u8>) -> (Result<u64, RejectReason>, Logged) {
    static LOGGER: CountingLogger = CountingLogger;
    // Each test installs the same logger; only the first install in a process takes.
    let _ignore = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    let (mut test, address) = load(code);
    LOGGED.with(|logged| logged.set(Logged::default()));
    let result = call_main(&mut test, address);
    (result, LOGGED.with(Cell::get))
}

/// Offset and length of an all-zero, valid UTF-8 run in [`template_module`]'s memory, larger than
/// any panic message the engine keeps.
const OVERSIZED_PANIC: (u32, u32) = (65536, 131072);

const ON_PANIC_IMPORT: &str = r#"(import "env" "on_panic" (func $on_panic (param i32 i32 i32 i32)))"#;

/// `on_panic` is a host call rather than a trap, so a template is free to make it any number of
/// times. Only the first report of an invocation is kept, and none of them is written to the log
/// at the levels a validator runs with.
#[test]
fn repeated_panic_reports_keep_the_first_and_are_not_logged() {
    let (offset, len) = OVERSIZED_PANIC;
    let code = template_module_importing(
        ON_PANIC_IMPORT,
        &format!(
            r#"
        (data (i32.const 64) "first")
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32)
          (local $i i32)
          (call $on_panic (i32.const 64) (i32.const 5) (i32.const 1) (i32.const 1))
          (local.set $i (i32.const 200))
          (loop $again
            (call $on_panic (i32.const {offset}) (i32.const {len}) (i32.const 2) (i32.const 1))
            (local.set $i (i32.sub (local.get $i) (i32.const 1)))
            (br_if $again (local.get $i)))
          unreachable)
        "#
        ),
    );

    let (result, logged) = call_logged(code);
    let reason = result.expect_err("a trapping call was accepted");
    let RejectReason::ExecutionFailure { message, .. } = reason else {
        panic!("expected an execution failure, got {reason:?}");
    };
    assert!(
        message.contains("first"),
        "the first panic was not kept: {message:.200}"
    );
    assert!(
        message.len() < 1024,
        "a later panic replaced the first: {} bytes",
        message.len()
    );
    assert!(
        logged.bytes < 4096,
        "{} bytes logged across {} records",
        logged.bytes,
        logged.records
    );
}

/// A panic message is truncated before the engine reads it, so an oversized one never reaches the
/// transaction result or the log at full size.
#[test]
fn an_oversized_panic_message_is_truncated() {
    let (offset, len) = OVERSIZED_PANIC;
    let code = template_module_importing(
        ON_PANIC_IMPORT,
        &format!(
            r#"
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32)
          (call $on_panic (i32.const {offset}) (i32.const {len}) (i32.const 1) (i32.const 1))
          unreachable)
        "#
        ),
    );

    let (result, logged) = call_logged(code);
    let reason = result.expect_err("a trapping call was accepted");
    let RejectReason::ExecutionFailure { message, .. } = reason else {
        panic!("expected an execution failure, got {reason:?}");
    };
    let limit = limits::ENGINE_LIMITS.max_panic_message_size;
    assert!(
        message.len() <= limit + 1024,
        "a {} byte panic message was kept",
        message.len()
    );
    assert!(
        logged.bytes <= 2 * limit,
        "{} bytes logged across {} records",
        logged.bytes,
        logged.records
    );
}

const TARI_ENGINE_IMPORT: &str = r#"(import "env" "tari_engine" (func $tari_engine (param i32 i32 i32) (result i32)))"#;

/// A template that ignores the null a failed engine call returns can go on making calls that fail.
/// Any one of them fails the transaction, and only the first is logged.
#[test]
fn repeated_failed_engine_calls_are_logged_once() {
    let code = template_module_importing(
        TARI_ENGINE_IMPORT,
        r#"
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32)
          (local $i i32)
          (local.set $i (i32.const 200))
          (loop $again
            (drop (call $tari_engine (i32.const 0x7fff) (i32.const 0) (i32.const 0)))
            (local.set $i (i32.sub (local.get $i) (i32.const 1)))
            (br_if $again (local.get $i)))
          (i32.const 20))
        "#,
    );

    let (result, logged) = call_logged(code);
    result.expect_err("a call whose engine calls failed was accepted");
    assert!(logged.records < 10, "{} records logged", logged.records);
}

/// A template log entry is echoed to the validator's log only once the engine has retained it, so
/// entries refused for their size or number are not written out either.
#[test]
fn refused_template_log_entries_are_not_logged() {
    use tari_template_lib::{args::EmitLogArg, types::LogLevel};

    let arg = tari_bor::encode(&EmitLogArg {
        message: "x".repeat(2 * limits::ENGINE_LIMITS.max_log_size_bytes),
        level: LogLevel::Error,
    })
    .unwrap();
    let arg_len = arg.len();
    let code = template_module_importing(
        TARI_ENGINE_IMPORT,
        &format!(
            r#"
        (data (i32.const 65536) "{}")
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32)
          (local $i i32)
          (local.set $i (i32.const 200))
          (loop $again
            (drop (call $tari_engine (i32.const 0) (i32.const 65536) (i32.const {arg_len})))
            (local.set $i (i32.sub (local.get $i) (i32.const 1)))
            (br_if $again (local.get $i)))
          (i32.const 20))
        "#,
            wat_bytes(&arg)
        ),
    );

    let (result, logged) = call_logged(code);
    result.expect_err("a call whose log entries were refused was accepted");
    assert!(
        logged.bytes < 4096,
        "{} bytes logged across {} records",
        logged.bytes,
        logged.records
    );
}

/// Once an engine call has failed, the invocation is lost, so the engine dispatches nothing the
/// template calls after it: valid log entries emitted after a refused one are neither retained nor
/// echoed, and the first failure stays the reject reason.
#[test]
fn no_engine_call_is_dispatched_after_one_has_failed() {
    use tari_template_lib::{args::EmitLogArg, types::LogLevel};

    let encode = |message: String| {
        tari_bor::encode(&EmitLogArg {
            message,
            level: LogLevel::Error,
        })
        .unwrap()
    };
    let oversized = encode("x".repeat(limits::ENGINE_LIMITS.max_log_size_bytes + 1));
    let valid = encode("after the refusal".to_string());
    let (oversized_len, valid_len) = (oversized.len(), valid.len());
    let code = template_module_importing(
        TARI_ENGINE_IMPORT,
        &format!(
            r#"
        (data (i32.const 4096) "{}")
        (data (i32.const 65536) "{}")
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32)
          (local $i i32)
          (drop (call $tari_engine (i32.const 0) (i32.const 65536) (i32.const {oversized_len})))
          (local.set $i (i32.const 200))
          (loop $again
            (drop (call $tari_engine (i32.const 0) (i32.const 4096) (i32.const {valid_len})))
            (local.set $i (i32.sub (local.get $i) (i32.const 1)))
            (br_if $again (local.get $i)))
          (i32.const 20))
        "#,
            wat_bytes(&valid),
            wat_bytes(&oversized)
        ),
    );

    let (result, logged) = call_logged(code);
    let reason = result.expect_err("a call whose first log entry was refused was accepted");
    assert!(
        reason.to_string().contains("Log entry of"),
        "the reject reason is not the first failure: {reason}"
    );
    assert!(logged.records < 10, "{} records logged", logged.records);
}
