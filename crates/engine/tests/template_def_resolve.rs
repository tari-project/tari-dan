//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Resolving a template's definition during execution is host work the meter does not see, so it must not scale
//! with the size of the definition. The engine resolves a definition on every function call, method call and
//! component creation; each of those must borrow the definition the template provider already holds rather than
//! copy it.
//!
//! Copies are observed by counting heap allocations on the executing thread: a definition carrying a nested
//! `Type::Tuple` tree costs one allocation per tuple node to copy, so a transaction against a template with a large
//! tree allocates no more than one against a template with a trivial definition only if nothing copies it.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

use tari_engine::wasm::WasmModule;
use tari_engine_types::hashing::hash_template_code;
use tari_ootle_common_types::substate_type::SubstateType;
use tari_ootle_transaction::{Epoch, Transaction, TransactionBuilder, args};
use tari_template_abi::{ArgDef, FunctionDef, TemplateDef, TemplateDefV1, Type};
use tari_template_lib::{
    args::{ComponentAction, ComponentInvokeArg, ComponentRef, CreateComponentArg},
    prelude::ComponentAccessRules,
    types::{OwnerRule, TemplateAddress, bytes::Bytes},
};
use tari_template_test_tooling::{Package, TemplateTest};

struct CountingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ignore = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn allocations() -> u64 {
    ALLOCATIONS.with(|n| n.get())
}

/// Component creations each call to the template performs.
const CREATES: u32 = 32;

/// Depth of the `Type::Tuple` binary tree; a copy of it makes `2^TREE_DEPTH - 1` allocations.
const TREE_DEPTH: usize = 10;

fn tuple_tree(depth: usize) -> Type {
    if depth == 0 {
        Type::Unit
    } else {
        Type::Tuple(vec![tuple_tree(depth - 1), tuple_tree(depth - 1)])
    }
}

/// A definition with a function `run` and a component method `poke`, both returning `output`.
fn template_def(template_name: &str, output: Type) -> TemplateDef {
    TemplateDef::V1(TemplateDefV1 {
        template_name: template_name.to_string(),
        abi_version: 0,
        functions: vec![
            FunctionDef {
                name: "run".to_string(),
                arguments: vec![],
                output: output.clone(),
                is_mut: false,
                is_migration: false,
            },
            FunctionDef {
                name: "poke".to_string(),
                arguments: vec![ArgDef {
                    name: "self".to_string(),
                    arg_type: Type::Other {
                        name: "&self".to_string(),
                    },
                }],
                output,
                is_mut: false,
                is_migration: false,
            },
        ],
    })
}

/// Renders bytes as a WAT string literal.
fn wat_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

/// A hand-written template whose entrypoint, whatever function is called, creates `CREATES` components that allow
/// any caller and returns unit. The declared return types are never checked against the returned value.
fn creator_module(template_name: &str, def: &TemplateDef) -> Vec<u8> {
    let create_arg = tari_bor::encode(&CreateComponentArg {
        encoded_state: tari_bor::Value::Null,
        owner_rule: OwnerRule::None,
        access_rules: ComponentAccessRules::allow_all(),
        address_allocation: None,
    })
    .unwrap();
    let invoke_arg = tari_bor::encode(&ComponentInvokeArg {
        component_ref: ComponentRef::Component,
        action: ComponentAction::Create,
        args: vec![Bytes::from_vec(create_arg)],
    })
    .unwrap();

    // `tari_alloc` hands out one fixed region at 1024 for the engine's `CallInfo`; the invoke argument sits at 64
    // and the encoded unit return value at 16 behind its 4-byte length prefix.
    let wat = format!(
        r#"
        (module
          (import "env" "tari_engine" (func (param i32 i32 i32) (result i32)))
          (memory (export "memory") 3)
          (data (i32.const 16) "\05\00\00\00\80")
          (data (i32.const 64) "{arg}")
          (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "tari_free") (param i32))
          (func (export "{template_name}_main") (param i32 i32) (result i32) (local i32)
            (block
              (loop
                (br_if 1 (i32.ge_u (local.get 2) (i32.const {CREATES})))
                (drop (call 0 (i32.const 1) (i32.const 64) (i32.const {arg_len})))
                (local.set 2 (i32.add (local.get 2) (i32.const 1)))
                (br 0)
              )
            )
            (i32.const 20)
          )
          (@custom "tari_tdef" "{tdef}")
        )
        "#,
        arg = wat_bytes(&invoke_arg),
        arg_len = invoke_arg.len(),
        tdef = wat_bytes(&def.encode_for_wasm_embedding().unwrap()),
    );
    let code = wat::parse_str(&wat).unwrap();
    WasmModule::validate_code(&code).expect("template must pass publish admission");
    code
}

struct Measured {
    run: u64,
    poke: u64,
}

/// Calls `run` and then `poke` on a component `run` created, each twice so that loading and compiling the template
/// fall outside the second measurement, and returns the allocations of each second call.
fn measure(test: &mut TemplateTest, template: TemplateAddress) -> Measured {
    // Each transaction needs a distinct nonce: the components it creates are addressed by its id.
    let execute = |test: &mut TemplateTest, nonce: u64, call: &dyn Fn(TransactionBuilder) -> TransactionBuilder| {
        let tx = call(Transaction::builder_localnet(Epoch(1)).with_nonce(nonce))
            .finish()
            .seal(test.secret_key());
        let before = allocations();
        test.execute_expect_success(tx, vec![]);
        allocations() - before
    };

    let run = |builder: TransactionBuilder| builder.call_function(template, "run", args![]);
    execute(test, 1, &run);
    let run_allocations = execute(test, 2, &run);

    let component = test
        .get_previous_output_address(SubstateType::Component)
        .as_component_address()
        .unwrap();
    let poke = |builder: TransactionBuilder| builder.call_method(component, "poke", args![]);
    execute(test, 3, &poke);
    let poke_allocations = execute(test, 4, &poke);

    Measured {
        run: run_allocations,
        poke: poke_allocations,
    }
}

#[test]
fn resolving_a_template_definition_does_not_copy_it() {
    let small_code = creator_module("Small", &template_def("Small", Type::Unit));
    let big_code = creator_module("Big", &template_def("Big", tuple_tree(TREE_DEPTH)));

    let small_addr = hash_template_code(&small_code);
    let big_addr = hash_template_code(&big_code);
    let mut builder = Package::builder();
    builder.add_all_builtin_templates();
    builder.add_template_from_code(small_addr, small_code).unwrap();
    builder.add_template_from_code(big_addr, big_code).unwrap();
    let mut test = TemplateTest::from_package(builder.build());

    let small = measure(&mut test, small_addr);
    let big = measure(&mut test, big_addr);

    // A single copy of the definition makes at least one allocation per tuple node.
    let one_copy = (1u64 << TREE_DEPTH) - 1;
    let run_excess = big.run.saturating_sub(small.run);
    let poke_excess = big.poke.saturating_sub(small.poke);
    assert!(
        run_excess < one_copy,
        "calling `run` ({CREATES} creates) allocated {run_excess} more with the large definition than with the small \
         one; a single copy of it is {one_copy} allocations"
    );
    assert!(
        poke_excess < one_copy,
        "calling `poke` ({CREATES} creates) allocated {poke_excess} more with the large definition than with the \
         small one; a single copy of it is {one_copy} allocations"
    );
}
