//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::{Duration, Instant};

use tari_ootle_transaction::{args, builder::named_args::NamedArg};
use tari_template_lib::types::ComponentAddress;
use tari_template_test_tooling::{ExecutionPoints, TemplateTest};

const TEMPLATE_PATHS: &[&str] = &["tests/templates/state_rewrite"];
const TEMPLATE_NAME: &str = "StateRewrite";
const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

const N: u32 = 3_000;
const TIMES: u32 = 40;

/// The fastest of a few runs, so that one run slowed by other work on the machine does not decide the
/// comparison.
fn timed_call(
    test: &mut TemplateTest,
    component: ComponentAddress,
    method: &str,
    args: Vec<NamedArg>,
) -> (Duration, ExecutionPoints) {
    let fastest = (0..3)
        .map(|_| {
            let start = Instant::now();
            test.call_method::<()>(component, method, args.clone(), vec![]);
            start.elapsed()
        })
        .min()
        .unwrap();
    (fastest, test.last_execution_points())
}

/// Rewrites a state of `N` references that the previous state already held, as the baseline the other
/// rewrites are compared with.
fn kept_rewrite(test: &mut TemplateTest) -> (Duration, ExecutionPoints) {
    let kept: ComponentAddress = test.call_function(TEMPLATE_NAME, "new", args![N], vec![]);
    timed_call(test, kept, "rewrite", args![false, TIMES])
}

/// Validating a state write is charged by the size of the state, so its host time must not depend on
/// how the written references relate to the previous state's. Both rewrites write the same number of
/// references and bytes, and are charged the same; one replaces every reference on each write, the
/// other writes only references the previous state held.
#[test]
fn state_write_validation_is_linear_in_references() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (kept_elapsed, kept_points) = kept_rewrite(&mut test);

    let replaced: ComponentAddress = test.call_function(TEMPLATE_NAME, "new", args![N], vec![]);
    let (replaced_elapsed, replaced_points) = timed_call(&mut test, replaced, "rewrite", args![true, TIMES]);
    eprintln!("kept: {kept_elapsed:.2?} {kept_points:?}, replaced: {replaced_elapsed:.2?} {replaced_points:?}");

    assert_eq!(kept_points.native, replaced_points.native);
    assert!(
        replaced_elapsed < kept_elapsed * 3,
        "rewriting {N} references {TIMES} times took {replaced_elapsed:.2?} when every reference was new, against \
         {kept_elapsed:.2?} when none was, for the same charge"
    );
}

/// Each write of a component's vaults checks that every previous vault is still held, so a state of
/// `N` vaults must validate in time comparable to a state of `N` resource references, which is charged
/// the same.
#[test]
fn state_write_validation_is_linear_in_vaults() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (kept_elapsed, kept_points) = kept_rewrite(&mut test);

    let vaults: ComponentAddress = test.call_function(TEMPLATE_NAME, "new", args![0u32], vec![]);
    // A transaction may create at most `max_substate_outputs` substates.
    for _ in 0..4 {
        test.call_method::<()>(vaults, "add_vaults", args![N / 4], vec![]);
    }
    let (reversed_elapsed, reversed_points) = timed_call(&mut test, vaults, "reverse_vaults", args![TIMES]);
    eprintln!("kept: {kept_elapsed:.2?} {kept_points:?}, reversed: {reversed_elapsed:.2?} {reversed_points:?}");

    assert_eq!(kept_points.native, reversed_points.native);
    assert!(
        reversed_elapsed < kept_elapsed * 3,
        "rewriting {N} vaults {TIMES} times took {reversed_elapsed:.2?}, against {kept_elapsed:.2?} for {N} resource \
         references"
    );
}
