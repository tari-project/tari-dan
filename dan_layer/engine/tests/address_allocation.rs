//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_engine::runtime::TransactionCommitError;
use tari_template_lib::{args, models::ComponentAddress};
use tari_template_test_tooling::{support::assert_error::assert_reject_reason, TemplateTest};
use tari_transaction::Transaction;

#[test]
fn it_uses_allocation_address() {
    let mut test = TemplateTest::new(["tests/templates/address_allocation"]);

    let result = test.execute_expect_success(
        Transaction::builder()
            .call_function(test.get_template_address("AddressAllocationTest"), "create", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );

    let actual = result
        .finalize
        .result
        .accept()
        .unwrap()
        .up_iter()
        .find_map(|(k, _)| k.as_component_address())
        .unwrap();

    let allocated = result.finalize.execution_results[0]
        .indexed
        .get_value::<ComponentAddress>("$.1")
        .unwrap()
        .unwrap();
    assert_eq!(actual, allocated);
}

#[test]
fn it_fails_if_allocation_is_not_used() {
    let mut test = TemplateTest::new(["tests/templates/address_allocation"]);
    let template_addr = test.get_template_address("AddressAllocationTest");

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_function(template_addr, "drop_allocation", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );

    assert_reject_reason(reason, TransactionCommitError::DanglingAddressAllocations { count: 1 });
}
