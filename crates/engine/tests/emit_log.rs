//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! What a template's `emit_log` calls cost the validator's own log. Every module here is
//! hand-written: a template built with `tari_template_lib` panics on the first refused engine call,
//! and these tests are about a template that ignores the refusal and keeps calling.

use std::sync::{Mutex, MutexGuard, Once};

use tari_engine_types::{
    commit_result::{RejectReason, TransactionResult},
    hashing::hash_template_code,
    limits,
};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::{args::EmitLogArg, types::LogLevel};
use tari_template_test_tooling::{Package, TemplateTest};

/// Same `TemplateDef` as `tests/wasm_loader.rs`: template `Buggy` with a unit-returning `main`.
const TEMPLATE_DEF: &[u8] = &[
    28, 0, 0, 0, 130, 0, 129, 131, 101, 66, 117, 103, 103, 121, 0, 129, 133, 100, 109, 97, 105, 110, 128, 130, 0, 128,
    244, 244,
];

const RUNTIME_LOG_TARGET: &str = "tari::ootle::engine::runtime";

/// Where the first encoded `EmitLogArg` is placed, clear of the fixed `tari_alloc` region at 1024
/// that receives each engine call's response.
const ARG_BASE: usize = 8192;

/// Byte length of every line written to [`RUNTIME_LOG_TARGET`] since the last [`capture_logs`].
static CAPTURED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// Serializes the tests in this file, which share the process-wide logger.
static SERIAL: Mutex<()> = Mutex::new(());
static INSTALL: Once = Once::new();

struct RuntimeLogCapture;

impl log::Log for RuntimeLogCapture {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.target() == RUNTIME_LOG_TARGET
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let len = record.args().to_string().len();
            CAPTURED.lock().unwrap().push(len);
        }
    }

    fn flush(&self) {}
}

static LOGGER: RuntimeLogCapture = RuntimeLogCapture;

fn capture_logs() -> MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    INSTALL.call_once(|| {
        log::set_logger(&LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
    CAPTURED.lock().unwrap().clear();
    guard
}

fn captured() -> Vec<usize> {
    CAPTURED.lock().unwrap().clone()
}

fn wat_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

fn log_arg(message: String) -> Vec<u8> {
    tari_bor::encode(&EmitLogArg {
        message,
        level: LogLevel::Error,
    })
    .unwrap()
}

/// A template whose `main` makes each `(arg, times)` `EmitLog` call `times` times in order, discarding
/// every response, and then returns unit.
fn emit_log_module(calls: &[(Vec<u8>, u32)]) -> Vec<u8> {
    let mut data = String::new();
    let mut body = String::new();
    let mut offset = ARG_BASE;
    for (arg, times) in calls {
        let len = arg.len();
        data.push_str(&format!("(data (i32.const {offset}) \"{}\")\n", wat_bytes(arg)));
        body.push_str(&format!(
            r#"
            (local.set $i (i32.const 0))
            (block $done
              (loop $emit
                (br_if $done (i32.ge_u (local.get $i) (i32.const {times})))
                (drop (call $tari_engine (i32.const 0) (i32.const {offset}) (i32.const {len})))
                (local.set $i (i32.add (local.get $i) (i32.const 1)))
                (br $emit)))
            "#
        ));
        offset += len;
    }

    let wat = format!(
        r#"
        (module
          (import "env" "tari_engine" (func $tari_engine (param i32 i32 i32) (result i32)))
          (memory (export "memory") 2)
          (data (i32.const 16) "\05\00\00\00\80")
          {data}
          (func (export "tari_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "tari_free") (param i32))
          (func (export "Buggy_main") (param i32 i32) (result i32)
            (local $i i32)
            {body}
            (i32.const 20))
          (@custom "tari_tdef" "{}")
        )
        "#,
        wat_bytes(TEMPLATE_DEF)
    );
    wat::parse_str(&wat).unwrap()
}

fn call_main(code: Vec<u8>) -> Result<(), RejectReason> {
    let address = hash_template_code(&code);
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
        TransactionResult::Accept(_) => Ok(()),
        TransactionResult::Reject(reason) | TransactionResult::AcceptFeeRejectRest(_, reason) => Err(reason),
    }
}

#[test]
fn an_oversized_entry_never_reaches_the_host_log() {
    let _guard = capture_logs();
    let oversized = log_arg("A".repeat(limits::ENGINE_LIMITS.max_log_size_bytes + 1));

    let reason = call_main(emit_log_module(&[(oversized, 4)])).expect_err("an oversized log entry was accepted");

    assert!(
        reason.to_string().contains("Log entry of"),
        "unexpected reject reason: {reason}"
    );
    assert_eq!(captured(), Vec::<usize>::new());
}

#[test]
fn entries_past_the_count_limit_never_reach_the_host_log() {
    let _guard = capture_logs();
    let max_logs = limits::ENGINE_LIMITS.max_logs;
    let message = "x".repeat(16);
    let calls = u32::try_from(max_logs).unwrap() + 16;

    let reason = call_main(emit_log_module(&[(log_arg(message.clone()), calls)]))
        .expect_err("logs past the limit were accepted");

    assert!(
        reason.to_string().contains("maximum number of logs"),
        "unexpected reject reason: {reason}"
    );
    let lines = captured();
    assert_eq!(lines.len(), max_logs);
    assert!(lines.iter().all(|len| *len == message.len()));
}

#[test]
fn no_engine_call_is_dispatched_after_one_has_failed() {
    let _guard = capture_logs();
    let oversized = log_arg("A".repeat(limits::ENGINE_LIMITS.max_log_size_bytes + 1));
    let small = log_arg("after the refusal".to_string());

    let reason = call_main(emit_log_module(&[(oversized, 1), (small, 4)])).expect_err("the invocation succeeded");

    assert!(
        reason.to_string().contains("Log entry of"),
        "the reject reason is not the first failure: {reason}"
    );
    assert_eq!(captured(), Vec::<usize>::new());
}
