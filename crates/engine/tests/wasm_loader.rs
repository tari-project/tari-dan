//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Admission rules the engine applies to an untrusted template binary, and the bounds it keeps over
//! one while it runs. Every module here is hand-written: the `#[template]` macro cannot express a
//! module that declares a start function, an oversized table, or a malformed ABI section, and those
//! are exactly the shapes a published binary may arrive in.

use tari_engine::wasm::{WasmExecutionError, WasmModule, WasmValidationError};
use tari_engine_types::{
    commit_result::{RejectReason, TransactionResult},
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
    let wat = format!(
        r#"
        (module
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
    let address: TemplateAddress = hash_template_code(&code);
    let mut builder = Package::builder();
    builder.add_all_builtin_templates();
    builder
        .add_template_from_code(address, code)
        .expect("template was rejected by the loader");
    let mut test = TemplateTest::from_package(builder.build());
    test.bootstrap_state();

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

/// A table without a declared maximum is capped by the engine rather than left to grow to whatever
/// a guest operand asks for: `table.grow` past the cap must refuse, returning -1.
#[test]
fn a_table_grows_no_further_than_the_limit() {
    let over_limit = limits::WASM_LIMITS.max_table_elements + 1;
    let code = template_module(&format!(
        r#"
        (table 1 funcref)
        (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
        (func (export "tari_free") (param i32))
        (func (export "Buggy_main") (param i32 i32) (result i32)
          (if (i32.ne (table.grow 0 (ref.null func) (i32.const {over_limit})) (i32.const -1))
            (then unreachable))
          (i32.const 20))
        "#
    ));

    load_and_call(code).expect("the call trapped: table.grow was allowed past the limit");
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
    let RejectReason::ExecutionFailure(error) = reason else {
        panic!("expected an execution failure, got {reason:?}");
    };
    assert!(
        error.contains(&limits::ENGINE_LIMITS.max_call_size.to_string()),
        "unexpected error: {error}"
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
