//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use tari_dan_engine::runtime::{LockError, LockState};
use tari_engine_types::lock::LockFlag;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress},
};
use tari_template_test_tooling::TemplateTest;
use tari_transaction::Transaction;

use crate::support::assert_error::assert_reject_reason;

#[test]
fn it_prevents_reentrant_withdraw() {
    let mut test = TemplateTest::new(["tests/templates/reentrancy", "tests/templates/faucet"]);
    let template_addr = test.get_template_address("Reentrancy");
    let faucet_addr = test.get_template_address("TestFaucet");
    let (account, _, _) = test.create_empty_account();

    let result = test.execute_expect_success(
        Transaction::builder()
            .call_function(faucet_addr, "mint", args![Amount(1000)])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );

    let faucet = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    let result = test.execute_expect_success(
        Transaction::builder()
            .call_method(faucet, "take_free_coins", args![])
            .put_last_instruction_output_on_workspace("bucket")
            .call_function(template_addr, "with_bucket", args![Workspace("bucket")])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );

    let reentrancy = result.finalize.execution_results[2]
        .decode::<ComponentAddress>()
        .unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_method(reentrancy, "get_balance", args![])
            .call_method(reentrancy, "reentrant_withdraw", args![Amount(1000)])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(reentrancy, "get_balance", args![])
            .call_method(account, "deposit", args![Workspace("bucket")])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );
    // Locked for read but attempted to lock the same component for write
    assert_reject_reason(reason, LockError::MultipleWriteLockRequested {
        address: reentrancy.into(),
    });
}

#[test]
fn it_allows_multiple_immutable_access_to_component() {
    let mut test = TemplateTest::new(["tests/templates/reentrancy"]);

    let reentrancy: ComponentAddress = test.call_function("Reentrancy", "new", args![], vec![]);

    test.execute_expect_success(
        Transaction::builder()
            .call_method(reentrancy, "reentrant_access_immutable", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );
}

#[test]
fn it_prevents_read_access_to_mutating_component() {
    let mut test = TemplateTest::new(["tests/templates/reentrancy"]);

    let reentrancy: ComponentAddress = test.call_function("Reentrancy", "new", args![], vec![]);

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_method(reentrancy, "reentrant_access", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );

    // Locked for read but attempted to lock the same component for write
    assert_reject_reason(reason, LockError::InvalidLockRequest {
        address: reentrancy.into(),
        requested_lock: LockFlag::Read,
        lock_state: LockState::Write,
    });
}

#[test]
fn it_prevents_multiple_mutable_access_to_component() {
    let mut test = TemplateTest::new(["tests/templates/reentrancy"]);

    let reentrancy: ComponentAddress = test.call_function("Reentrancy", "new", args![], vec![]);

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_method(reentrancy, "reentrant_access_mut", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );

    // Locked for read but attempted to lock the same component for write
    assert_reject_reason(reason, LockError::MultipleWriteLockRequested {
        address: reentrancy.into(),
    });
}
