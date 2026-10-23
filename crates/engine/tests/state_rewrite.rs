//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::{Duration, Instant};

use tari_ootle_transaction::args;
use tari_template_lib::types::ComponentAddress;
use tari_template_test_tooling::{ExecutionPoints, TemplateTest};

const TEMPLATE_PATHS: &[&str] = &["tests/templates/state_rewrite"];
const TEMPLATE_NAME: &str = "StateRewrite";
const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

/// The fastest of a few runs, so that one run slowed by other work on the machine does not decide the
/// comparison.
fn timed_rewrite(
    test: &mut TemplateTest,
    component: ComponentAddress,
    replace_all: bool,
    times: u32,
) -> (Duration, ExecutionPoints) {
    let fastest = (0..3)
        .map(|_| {
            let start = Instant::now();
            test.call_method::<()>(component, "rewrite", args![replace_all, times], vec![]);
            start.elapsed()
        })
        .min()
        .unwrap();
    (fastest, test.last_execution_points())
}

/// Validating a state write is charged by the size of the state, so its host time must not depend on
/// how the written references relate to the previous state's. Both rewrites write the same number of
/// references and bytes, and are charged the same; one replaces every reference on each write, the
/// other writes only references the previous state held.
#[test]
fn state_write_validation_is_linear_in_references() {
    const N: u32 = 3_000;
    const TIMES: u32 = 40;

    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let kept: ComponentAddress = test.call_function(TEMPLATE_NAME, "new", args![N], vec![]);
    let replaced: ComponentAddress = test.call_function(TEMPLATE_NAME, "new", args![N], vec![]);

    let (kept_elapsed, kept_points) = timed_rewrite(&mut test, kept, false, TIMES);
    let (replaced_elapsed, replaced_points) = timed_rewrite(&mut test, replaced, true, TIMES);
    eprintln!("kept: {kept_elapsed:.2?} {kept_points:?}, replaced: {replaced_elapsed:.2?} {replaced_points:?}");

    assert_eq!(kept_points.native, replaced_points.native);
    assert!(
        replaced_elapsed < kept_elapsed * 3,
        "rewriting {N} references {TIMES} times took {replaced_elapsed:.2?} when every reference was new, against \
         {kept_elapsed:.2?} when none was, for the same charge"
    );
}
