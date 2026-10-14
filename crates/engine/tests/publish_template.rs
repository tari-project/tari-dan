// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::iter;

use ootle_byte_type::ToByteType;
use rand::random;
use tari_engine::transaction::TransactionErrorKind;
use tari_engine_types::{
    commit_result::{ExecutionFailureCode, RejectReason, TransactionResult},
    hashing::hash_template_code,
    limits,
    published_template::PublishedTemplateAddress,
    substate::{SubstateId, SubstateValue},
};
use tari_ootle_transaction::{Epoch, Transaction};
use tari_template_abi::TEMPLATE_DEF_CUSTOM_SECTION;
use tari_template_test_tooling::{
    TemplateTest,
    compile::compile_template,
    support::assert_error::{assert_reject_reason, assert_reject_reason_with_code},
};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn publish_template_success() {
    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account_address, owner_proof, account_key, public_key) = test.create_funded_account_with_keypair();
    let template = compile_template("tests/templates/hello_world", &[]).unwrap();
    let expected_binary_hash = hash_template_code(template.code());
    let expected_template_address =
        PublishedTemplateAddress::from_author_and_binary_hash(&public_key.to_byte_type(), &expected_binary_hash);

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account_address, 200_000u64)
            .publish_template(template.into_code())
            .build_and_seal(&account_key),
        vec![owner_proof],
    );

    assert!(matches!(result.finalize.result, TransactionResult::Accept(_)));

    let mut template_found = false;
    let diff = result.expect_success();
    diff.up_iter().for_each(|(substate_id, substate)| {
        if let SubstateValue::Template(curr_template) = substate.substate_value() &&
            curr_template.to_binary_hash() == expected_binary_hash
        {
            template_found = true;
            assert!(matches!(substate_id, SubstateId::Template(_)));
            if let SubstateId::Template(curr_template_addr) = substate_id {
                assert_eq!(expected_template_address, *curr_template_addr);
            }
        }
    });

    assert!(template_found);
}

#[test]
fn publish_template_invalid_binary() {
    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account_address, owner_proof, account_key, _) = test.create_funded_account_with_keypair();
    let result = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account_address, 200_000u64)
            // Main intent instruction #1
            .publish_template(vec![1u8, 2, 3])
            .build_and_seal(&account_key),
        vec![owner_proof],
    );

    let RejectReason::ExecutionFailure { code, message } = result else {
        panic!("expected an execution failure, got {result:?}");
    };
    assert_eq!(code, ExecutionFailureCode::TemplateError);
    assert!(message.starts_with("At instruction #1: Load template error:"));
}

#[test]
fn publish_template_too_big_binary() {
    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account_address, owner_proof, account_key, _) = test.create_funded_account_with_keypair();
    let random_wasm_binary = generate_random_binary(limits::ENGINE_LIMITS.max_template_binary_size_bytes + 1);
    let wasm_binary_size = random_wasm_binary.len();
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account_address, 200_000u64)
            .publish_template(random_wasm_binary)
            .build_and_seal(&account_key),
        vec![owner_proof],
    );

    assert_reject_reason(reason, TransactionErrorKind::WasmBinaryTooBig {
        size: wasm_binary_size,
        max: limits::ENGINE_LIMITS.max_template_binary_size_bytes,
    });
}

fn generate_random_binary(size_in_bytes: usize) -> Vec<u8> {
    iter::repeat_with(random).take(size_in_bytes).collect()
}

/// A template's ABI comes from its `tari_tdef` custom section, so a binary without one carries no
/// template definition the engine can admit — including one embedding its ABI the legacy way, in
/// linear memory behind an `_ABI_TEMPLATE_DEF` global.
#[test]
fn publish_template_without_a_template_def_section() {
    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account_address, owner_proof, account_key, _) = test.create_funded_account_with_keypair();

    let code = wat::parse_str(
        r#"
        (module
          (memory (export "memory") 1)
          (global (export "_ABI_TEMPLATE_DEF") i32 (i32.const -1)))
        "#,
    )
    .unwrap();

    let result = test.execute_expect_failure(
        test.transaction()
            .pay_fee_from_component(account_address, 200_000u64)
            .publish_template(code)
            .build_and_seal(&account_key),
        vec![owner_proof],
    );

    assert_reject_reason(result, TEMPLATE_DEF_CUSTOM_SECTION);
}

/// A publish makes every validator Cranelift-compile the binary it carries, which is the most
/// expensive thing one instruction can ask for. It is charged before the compile runs, so a
/// transaction that cannot cover it does none of the work.
#[test]
fn the_compile_a_publish_pays_for_is_charged_before_it_runs() {
    use tari_engine_types::limits::{CompileCounts, template_compile_points};

    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account, owner_proof, key, _) = test.create_funded_account_with_keypair();
    let template = compile_template("tests/templates/hello_world", &[]).unwrap();
    let binary_len = template.code().len() as u64;
    test.enable_fees();

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account, 2_000_000u64)
            .publish_template(template.into_code())
            .build_and_seal(&key),
        vec![owner_proof],
    );

    let native_points = result.native_execution_points;
    let compile = template_compile_points(binary_len, &CompileCounts::default());
    assert!(
        native_points >= compile,
        "a {binary_len}-byte publish charged {native_points} native points, under the {compile} its compile costs"
    );
}

/// Every admission rule the module bytes alone can answer runs before the compile is charged for,
/// so a module refused by one of them pays nothing for cranelift. Points accumulated before a
/// failure are still charged, so charging first would bill a large rejected binary for a compile
/// that never ran.
#[test]
fn a_module_refused_before_the_compile_is_not_charged_for_it() {
    use tari_engine_types::limits::{CompileCounts, template_compile_points};

    // Rejected by `validate_module_structure`, which needs no compile: the start section is refused
    // before cranelift is reached.
    let code = wat::parse_str(
        r#"
        (module
          (memory (export "memory") 3)
          (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "tari_free") (param i32))
          (func (export "Buggy_main") (param i32 i32) (result i32) (i32.const 20))
          (func)
          (start 3)
        )
        "#,
    )
    .unwrap();

    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account, owner_proof, key, _) = test.create_funded_account_with_keypair();
    test.enable_fees();

    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .pay_fee_from_component(account, 2_000_000u64)
                .publish_template(code.clone())
                .build_and_seal(&key),
            vec![owner_proof],
        )
        .expect("execution failed");

    assert!(
        result.native_execution_points < template_compile_points(code.len() as u64, &CompileCounts::default()),
        "a module refused before the compile was charged {} native points",
        result.native_execution_points
    );
}

/// A module of `count` empty functions: the most functions a binary of its size can hold.
fn module_of_empty_functions(count: usize) -> Vec<u8> {
    let mut wat = String::from("(module (type (func))");
    wat.extend(iter::repeat_n("(func (type 0))", count));
    wat.push(')');
    wat::parse_str(wat).unwrap()
}

/// Cranelift's cost per function is far above what an empty function's four bytes are priced at, so
/// a module dense in functions is billed by its function count rather than its size.
#[test]
fn a_module_dense_in_functions_is_charged_for_each_function() {
    use tari_engine_types::limits::{CompileCounts, template_compile_points};

    let functions = 1024;
    let code = module_of_empty_functions(functions);

    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account, owner_proof, key, _) = test.create_funded_account_with_keypair();
    test.enable_fees();

    // Refused after the compile for lacking a template definition, by which point the compile has
    // been charged.
    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .pay_fee_from_component(account, 2_000_000u64)
                .publish_template(code.clone())
                .build_and_seal(&key),
            vec![owner_proof],
        )
        .expect("execution failed");
    let (_, reason) = result
        .finalize
        .fee_accept_transaction_reject()
        .expect("the publish should fail after the fee is paid");
    assert_reject_reason(reason, TEMPLATE_DEF_CUSTOM_SECTION);

    // Each function's closing `end` is followed by its one metering check.
    let compile = template_compile_points(code.len() as u64, &CompileCounts {
        functions: functions as u64,
        variables: 0,
        metering_checks: functions as u64,
        largest_function_metering_checks: 1,
    });
    assert!(
        compile > template_compile_points(code.len() as u64, &CompileCounts::default()),
        "a {}-byte module of {functions} functions is billed no more than its size",
        code.len()
    );
    assert!(
        result.native_execution_points >= compile,
        "a publish of {functions} functions charged {} native points, under the {compile} its compile costs",
        result.native_execution_points
    );
}

/// The function cap is checked before the compile, so a module over it does no Cranelift work and
/// pays for none.
#[test]
fn a_module_with_too_many_functions_is_refused_before_the_compile() {
    use tari_engine_types::limits::{CompileCounts, WASM_LIMITS, template_compile_points};

    let code = module_of_empty_functions(WASM_LIMITS.max_module_functions + 1);

    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account, owner_proof, key, _) = test.create_funded_account_with_keypair();
    test.enable_fees();

    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .pay_fee_from_component(account, 2_000_000u64)
                .publish_template(code.clone())
                .build_and_seal(&key),
            vec![owner_proof],
        )
        .expect("execution failed");

    let (_, reason) = result
        .finalize
        .fee_accept_transaction_reject()
        .expect("the publish should fail after the fee is paid");
    assert_reject_reason_with_code(
        reason,
        format!(
            "Module contains more than {} functions",
            WASM_LIMITS.max_module_functions
        ),
        ExecutionFailureCode::TemplateError,
    );
    assert!(
        result.native_execution_points < template_compile_points(code.len() as u64, &CompileCounts::default()),
        "a module refused for its function count was charged {} native points",
        result.native_execution_points
    );
}

/// A module of `functions` functions each declaring `locals` locals, in a single `(count, type)` run
/// of a few bytes. Valid while `locals` is within wasmparser's per-function limit of 50,000.
fn module_of_functions_with_locals(functions: usize, locals: usize) -> Vec<u8> {
    let func = format!("(func (local{}))", " i64".repeat(locals));
    let mut wat = String::from("(module");
    wat.extend(iter::repeat_n(func.as_str(), functions));
    wat.push(')');
    wat::parse_str(wat).unwrap()
}

/// Cranelift declares every parameter and local as a variable of its own, which the few bytes of a
/// `(count, type)` run do not pay for, so a module dense in them is billed by their count.
#[test]
fn a_module_dense_in_locals_is_charged_for_each_local() {
    use tari_engine_types::limits::{CompileCounts, template_compile_points};

    let functions = 8;
    let locals = functions * 25_000;
    let code = module_of_functions_with_locals(functions, locals / functions);

    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account, owner_proof, key, _) = test.create_funded_account_with_keypair();
    test.enable_fees();

    // Refused after the compile for lacking a template definition, by which point the compile has
    // been charged.
    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .pay_fee_from_component(account, 2_000_000u64)
                .publish_template(code.clone())
                .build_and_seal(&key),
            vec![owner_proof],
        )
        .expect("execution failed");
    let (_, reason) = result
        .finalize
        .fee_accept_transaction_reject()
        .expect("the publish should fail after the fee is paid");
    assert_reject_reason(reason, TEMPLATE_DEF_CUSTOM_SECTION);

    let compile = template_compile_points(code.len() as u64, &CompileCounts {
        functions: functions as u64,
        variables: locals as u64,
        metering_checks: functions as u64,
        largest_function_metering_checks: 1,
    });
    assert!(
        compile > template_compile_points(code.len() as u64, &CompileCounts::default()),
        "a {}-byte module of {locals} locals is billed no more than its size",
        code.len()
    );
    assert!(
        result.native_execution_points >= compile,
        "a publish of {locals} locals charged {} native points, under the {compile} its compile costs",
        result.native_execution_points
    );
}

/// The variable cap is checked before the compile, so a module over it does no Cranelift work and
/// pays for none.
#[test]
fn a_module_with_too_many_locals_is_refused_before_the_compile() {
    use tari_engine_types::limits::{CompileCounts, WASM_LIMITS, template_compile_points};

    let locals_per_function = 25_000;
    let functions = WASM_LIMITS.max_module_variables / locals_per_function + 1;
    let code = module_of_functions_with_locals(functions, locals_per_function);

    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account, owner_proof, key, _) = test.create_funded_account_with_keypair();
    test.enable_fees();

    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .pay_fee_from_component(account, 2_000_000u64)
                .publish_template(code.clone())
                .build_and_seal(&key),
            vec![owner_proof],
        )
        .expect("execution failed");

    let (_, reason) = result
        .finalize
        .fee_accept_transaction_reject()
        .expect("the publish should fail after the fee is paid");
    assert_reject_reason_with_code(
        reason,
        format!(
            "Module's functions declare more than {} parameters and locals",
            WASM_LIMITS.max_module_variables
        ),
        ExecutionFailureCode::TemplateError,
    );
    assert!(
        result.native_execution_points < template_compile_points(code.len() as u64, &CompileCounts::default()),
        "a module refused for its local count was charged {} native points",
        result.native_execution_points
    );
}

/// A module of `functions` functions whose body is `body`.
fn module_of_functions(functions: usize, body: &str) -> Vec<u8> {
    let func = format!("(func {body})");
    let mut wat = String::from("(module");
    wat.extend(iter::repeat_n(func.as_str(), functions));
    wat.push(')');
    wat::parse_str(wat).unwrap()
}

/// Publishes `code` with fees on, expecting it refused with `error` after the fee is paid and
/// before the compile is charged for.
fn assert_refused_before_the_compile(code: Vec<u8>, error: String) {
    use tari_engine_types::limits::{CompileCounts, template_compile_points};

    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account, owner_proof, key, _) = test.create_funded_account_with_keypair();
    test.enable_fees();

    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .pay_fee_from_component(account, 2_000_000u64)
                .publish_template(code.clone())
                .build_and_seal(&key),
            vec![owner_proof],
        )
        .expect("execution failed");

    let (_, reason) = result
        .finalize
        .fee_accept_transaction_reject()
        .expect("the publish should fail after the fee is paid");
    assert_reject_reason_with_code(reason, error, ExecutionFailureCode::TemplateError);
    assert!(
        result.native_execution_points < template_compile_points(code.len() as u64, &CompileCounts::default()),
        "a module refused before the compile was charged {} native points",
        result.native_execution_points
    );
}

/// Cranelift's time on a function grows faster than linearly in its `loop`s, so the loop cap is
/// checked before the compile.
#[test]
fn a_function_with_too_many_loops_is_refused_before_the_compile() {
    use tari_engine_types::limits::WASM_LIMITS;

    let max_loops = WASM_LIMITS.max_function_loops;
    assert_refused_before_the_compile(
        module_of_functions(1, &"(loop)".repeat(max_loops + 1)),
        format!("Function 0 contains more than {max_loops} loops"),
    );
}

/// Wasmer parallelises a compile across runs of function bytes, so a module of loops, three bytes
/// each, compiles nearly serially however its loops are spread across functions.
#[test]
fn a_module_with_too_many_loops_is_refused_before_the_compile() {
    use tari_engine_types::limits::WASM_LIMITS;

    let max_loops = WASM_LIMITS.max_module_loops;
    let loops_per_function = WASM_LIMITS.max_function_loops;
    let functions = max_loops / loops_per_function + 1;
    assert_refused_before_the_compile(
        module_of_functions(functions, &"(loop)".repeat(loops_per_function)),
        format!("Module contains more than {max_loops} loops"),
    );
}

/// A metering check is compiled after each control-flow operator, and a function is compiled on
/// one thread at a cost that grows faster than linearly in its checks.
#[test]
fn a_function_with_too_many_control_flow_operators_is_refused_before_the_compile() {
    use tari_engine_types::limits::WASM_LIMITS;

    let max_checks = WASM_LIMITS.max_function_metering_checks;
    // Each block's `end` is followed by a check, and the function's own closing `end` by one more.
    assert_refused_before_the_compile(
        module_of_functions(1, &"(block)".repeat(max_checks)),
        format!("Function 0 contains more than {max_checks} control-flow and bulk-memory operators"),
    );
}

/// The bulk meter compiles a check of its own after each bulk-memory operator, which counts
/// towards the same limit.
#[test]
fn a_function_with_too_many_bulk_memory_operators_is_refused_before_the_compile() {
    use tari_engine_types::limits::WASM_LIMITS;

    let max_checks = WASM_LIMITS.max_function_metering_checks;
    let fill = "(memory.fill (i32.const 0) (i32.const 0) (i32.const 0))".repeat(max_checks);
    let code = wat::parse_str(format!("(module (memory 1) (func {fill}))")).unwrap();
    assert_refused_before_the_compile(
        code,
        format!("Function 0 contains more than {max_checks} control-flow and bulk-memory operators"),
    );
}

#[test]
fn a_function_nested_too_deep_is_refused_before_the_compile() {
    use tari_engine_types::limits::WASM_LIMITS;

    let max_depth = WASM_LIMITS.max_function_nesting_depth;
    let body = format!("{}{}", "(block ".repeat(max_depth + 1), ")".repeat(max_depth + 1));
    assert_refused_before_the_compile(
        module_of_functions(1, &body),
        format!("Function 0 nests blocks more than {max_depth} deep"),
    );
}

/// No function exceeds a limit here, but together they ask for more compile work than a binary at
/// the size cap is billed for, from a binary a fraction of that size.
#[test]
fn a_module_asking_more_compile_work_than_the_size_cap_is_refused_before_the_compile() {
    use tari_engine_types::limits::{CompileCounts, ENGINE_LIMITS, WASM_LIMITS, template_compile_count_bytes};

    let max_checks = WASM_LIMITS.max_function_metering_checks;
    let functions = 8;
    // A one-byte `return` is the smallest operator a check follows.
    let code = module_of_functions(functions, &"(return)".repeat(max_checks - 1));
    let billed_bytes = template_compile_count_bytes(&CompileCounts {
        functions: functions as u64,
        variables: 0,
        metering_checks: (functions * max_checks) as u64,
        largest_function_metering_checks: max_checks as u64,
    });
    assert!(code.len() < ENGINE_LIMITS.max_template_binary_size_bytes / 8);

    assert_refused_before_the_compile(
        code,
        format!(
            "Module asks the compile for the work of a {billed_bytes}-byte binary, more than the {}-byte publish limit",
            ENGINE_LIMITS.max_template_binary_size_bytes
        ),
    );
}

/// A function is compiled on one thread at a cost that grows with the square of its metering
/// checks, so a module is billed at least that for its largest function.
#[test]
fn a_large_function_is_charged_for_compiling_on_one_thread() {
    use tari_engine_types::limits::{CompileCounts, template_compile_points};

    let checks = 4_000;
    let code = module_of_functions(1, &"(return)".repeat(checks - 1));

    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account, owner_proof, key, _) = test.create_funded_account_with_keypair();
    test.enable_fees();

    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .pay_fee_from_component(account, 2_000_000u64)
                .publish_template(code.clone())
                .build_and_seal(&key),
            vec![owner_proof],
        )
        .expect("execution failed");
    let (_, reason) = result
        .finalize
        .fee_accept_transaction_reject()
        .expect("the publish should fail after the fee is paid");
    assert_reject_reason(reason, TEMPLATE_DEF_CUSTOM_SECTION);

    let summed = CompileCounts {
        functions: 1,
        variables: 0,
        metering_checks: checks as u64,
        largest_function_metering_checks: 0,
    };
    let compile = template_compile_points(code.len() as u64, &CompileCounts {
        largest_function_metering_checks: checks as u64,
        ..summed
    });
    assert!(
        compile > template_compile_points(code.len() as u64, &summed),
        "a function of {checks} metering checks is billed no more than their sum"
    );
    assert!(
        result.native_execution_points >= compile,
        "a publish of one function of {checks} metering checks charged {} native points, under the {compile} its \
         compile costs",
        result.native_execution_points
    );
}

/// The operator a metering check follows can be a single byte, far less than the check's compile
/// costs at the byte price, so a module dense in them is billed by their count rather than its
/// size.
#[test]
fn a_module_dense_in_metering_checks_is_charged_for_each_check() {
    use tari_engine_types::limits::{CompileCounts, template_compile_points};

    let functions = 16;
    let checks_per_function = 2_000;
    let code = module_of_functions(functions, &"(return)".repeat(checks_per_function - 1));

    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    let (account, owner_proof, key, _) = test.create_funded_account_with_keypair();
    test.enable_fees();

    // Refused after the compile for lacking a template definition, by which point the compile has
    // been charged.
    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .pay_fee_from_component(account, 2_000_000u64)
                .publish_template(code.clone())
                .build_and_seal(&key),
            vec![owner_proof],
        )
        .expect("execution failed");
    let (_, reason) = result
        .finalize
        .fee_accept_transaction_reject()
        .expect("the publish should fail after the fee is paid");
    assert_reject_reason(reason, TEMPLATE_DEF_CUSTOM_SECTION);

    let compile = template_compile_points(code.len() as u64, &CompileCounts {
        functions: functions as u64,
        variables: 0,
        metering_checks: (functions * checks_per_function) as u64,
        largest_function_metering_checks: checks_per_function as u64,
    });
    assert!(
        compile > template_compile_points(code.len() as u64, &CompileCounts::default()),
        "a {}-byte module of {} metering checks is billed no more than its size",
        code.len(),
        functions * checks_per_function
    );
    assert!(
        result.native_execution_points >= compile,
        "a publish of {} metering checks charged {} native points, under the {compile} its compile costs",
        functions * checks_per_function,
        result.native_execution_points
    );
}
