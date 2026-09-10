//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::BTreeMap;

use tari_engine::runtime::RuntimeError;
use tari_engine_types::indexed_value::IndexedWellKnownTypes;
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::{
    args::VaultFreezeFlag,
    types::{ComponentAddress, ResourceAddress, VaultId, constants::TARI_TOKEN},
};
use tari_template_test_tooling::{TemplateTest, support::assert_error::assert_reject_reason};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn it_freezes_vaults_containing_a_freezable_resource() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/freeze"]);
    let template = test.get_template_address("Freeze");
    let (account, account_proof, _) = test.create_empty_account();

    // Create a new Freeze component and deposit some resources into the account
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .allocate_component_address("freeze_comp")
            .call_function(template, "new", args![Workspace("freeze_comp")])
            .call_method("freeze_comp", "withdraw", args![1000])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(account, "deposit", args![Workspace("bucket")])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    let component: ComponentAddress = result.finalize.execution_results[1].get_value("$.0").unwrap().unwrap();
    let resource: ResourceAddress = result.finalize.execution_results[1].get_value("$.1").unwrap().unwrap();
    // Account fields: 0=vaults, 1=approvals.
    let vaults: BTreeMap<ResourceAddress, VaultId> = test.extract_component_value(account, "$.0");
    let vault_id = vaults[&resource];

    // Freeze the account's vault
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(component, "freeze", args![vault_id])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    // Attempt to withdraw from the frozen vault - FAIL
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(account, "withdraw", args![resource, 10])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(account, "deposit", args![Workspace("bucket")])
            .build_and_seal(test.secret_key()),
        vec![account_proof.clone()],
    );

    assert_reject_reason(reason, RuntimeError::VaultFrozen {
        vault_id,
        freeze_flag: VaultFreezeFlag::Withdrawals,
    });

    // Unfreeze the vault
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(component, "unfreeze", args![vault_id])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    // Withdraw from the un-frozen vault - SUCCESS
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(account, "withdraw", args![resource, 10])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(account, "deposit", args![Workspace("bucket")])
            .build_and_seal(test.secret_key()),
        vec![account_proof],
    );
}

#[test]
fn it_refuses_to_freeze_a_vault_of_another_resource() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/freeze"]);
    let template = test.get_template_address("Freeze");
    let (account, _, _) = test.create_funded_account();

    let result = test.execute_expect_success(
        test.transaction()
            .allocate_component_address("freeze_comp")
            .call_function(template, "new", args![Workspace("freeze_comp")])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    let component: ComponentAddress = result.finalize.execution_results[1].get_value("$.0").unwrap().unwrap();
    let freeze_resource: ResourceAddress = result.finalize.execution_results[1].get_value("$.1").unwrap().unwrap();

    // The account's only vault holds TARI, not the freezable resource whose Freeze rule authorized the action.
    let vault_id = {
        let store = test.read_only_state_store();
        let component = store.get_component(account).unwrap();
        let values = IndexedWellKnownTypes::from_value(component.state()).unwrap();
        values.vault_ids()[0]
    };

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_method(component, "freeze", args![vault_id])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    assert_reject_reason(reason, RuntimeError::FreezeResourceMismatch {
        vault_id,
        resource_address: freeze_resource,
        vault_resource: TARI_TOKEN,
    });
}
