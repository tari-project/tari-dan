//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use tari_dan_engine::runtime::{LockError, RuntimeError};
use tari_engine_types::indexed_value::IndexedWellKnownTypes;
use tari_template_lib::{
    args,
    args::VaultAction,
    constants::XTR2,
    models::{Amount, ComponentAddress, ResourceAddress},
};
use tari_template_test_tooling::TemplateTest;
use tari_transaction::Transaction;

use crate::support::assert_error::assert_reject_reason;

#[test]
fn it_rejects_dangling_vaults_in_constructor() {
    let mut test = TemplateTest::new(["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address("Shenanigans");

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_function(template_addr, "dangling_vault", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );

    assert_reject_reason(
        reason,
        // TODO: should have the actual RuntimeError in the RejectReason
        "1 orphaned substate(s) detected",
    );
}

#[test]
fn it_rejects_dangling_vault_that_has_been_returned() {
    let mut test = TemplateTest::new(["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address("Shenanigans");

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_function(template_addr, "return_vault", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );

    assert_reject_reason(reason, "1 orphaned substate(s) detected");
}

#[test]
fn it_rejects_dangling_vaults_in_component() {
    let mut test = TemplateTest::new(["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address("Shenanigans");

    //  Create with vault
    let result = test.execute_expect_success(
        Transaction::builder()
            .call_function(template_addr, "with_vault", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );

    let component_address = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();
    let component = test.read_only_state_store().get_component(component_address).unwrap();
    let indexed = IndexedWellKnownTypes::from_value(component.state()).unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_method(component_address, "drop_vault", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![test.get_test_proof()],
    );

    assert_reject_reason(reason, RuntimeError::OrphanedSubstate {
        address: indexed.vault_ids()[0].into(),
    });
}

#[test]
fn it_rejects_dangling_resources() {
    let mut test = TemplateTest::new(["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address("Shenanigans");

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_function(template_addr, "dangling_resource", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );

    assert_reject_reason(reason, "1 orphaned substate(s) detected")
}

#[test]
fn it_rejects_unknown_substate_addresses() {
    let mut test = TemplateTest::new(["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address("Shenanigans");

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_function(template_addr, "non_existent_id", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![],
    );

    assert_reject_reason(reason, RuntimeError::SubstateOutOfScope {
        address: ResourceAddress::from_hex("abababababababababababababababababababababababababababababababab")
            .unwrap()
            .into(),
    })
}

#[test]
fn it_rejects_references_to_buckets_that_arent_in_scope() {
    let mut test = TemplateTest::new(["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address("Shenanigans");
    let (account, owner_token, owner_key) = test.create_owned_account();

    let result = test.execute_expect_success(
        Transaction::builder()
            .call_function(template_addr, "with_vault", args![])
            .sign(&owner_key)
            .build(),
        vec![owner_token.clone()],
    );

    let shenanigans = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_method(account, "withdraw", args![XTR2, Amount(1000)])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(shenanigans, "take_bucket_zero", args![])
            .sign(&owner_key)
            .build(),
        vec![owner_token],
    );

    // take_bucket_zero fails because the bucket isnt in scope for the call
    assert_reject_reason(reason, RuntimeError::BucketNotFound { bucket_id: 0.into() });
}

#[test]
fn it_rejects_double_ownership_of_vault() {
    let mut test = TemplateTest::new(["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address("Shenanigans");

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_function(template_addr, "with_vault_copy", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![test.get_test_proof()],
    );

    assert_reject_reason(reason, "Duplicate reference to substate");
}

#[test]
fn it_prevents_access_to_vault_id_in_component_context() {
    let mut test = TemplateTest::new(["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address("Shenanigans");
    let (account, _, _) = test.create_owned_account();

    let vault_id = {
        let component = test.read_only_state_store().get_component(account).unwrap();
        let values = IndexedWellKnownTypes::from_value(component.state()).unwrap();
        values.vault_ids()[0]
    };

    let result = test.execute_expect_success(
        Transaction::builder()
            .call_function(template_addr, "with_vault", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![test.get_test_proof()],
    );

    let shenanigans = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_method(shenanigans, "take_from_a_vault", args![vault_id, Amount(1000)])
            .sign(test.get_test_secret_key())
            .build(),
        vec![test.get_test_proof()],
    );

    // take_bucket_zero fails because the component didnt create the vault
    assert_reject_reason(reason, RuntimeError::SubstateNotOwned {
        address: vault_id.into(),
        requested_owner: shenanigans.into(),
    });
}

#[test]
fn it_prevents_access_to_out_of_scope_component() {
    let mut test = TemplateTest::new(["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address("Shenanigans");
    let (account, _, _) = test.create_owned_account();

    let result = test.execute_expect_success(
        Transaction::builder()
            .call_function(template_addr, "new", args![])
            .sign(test.get_test_secret_key())
            .build(),
        vec![test.get_test_proof()],
    );

    let shenanigans = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_method(shenanigans, "empty_state_on_component", args![account])
            .sign(test.get_test_secret_key())
            .build(),
        vec![test.get_test_proof()],
    );

    // Fails because the engine does not lock this component
    assert_reject_reason(
        reason,
        RuntimeError::LockError(LockError::SubstateNotLocked {
            address: account.into(),
        }),
    );
}

#[test]
fn it_disallows_calls_on_vaults_that_are_not_owned_by_current_component() {
    let mut test = TemplateTest::new(["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address("Shenanigans");
    let (victim, _, _) = test.create_owned_account();
    let (attacker, _, _) = test.create_empty_account();

    let vault_id = {
        let component = test.read_only_state_store().get_component(victim).unwrap();
        let values = IndexedWellKnownTypes::from_value(component.state()).unwrap();
        values.vault_ids()[0]
    };

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_function(
                template_addr,
                "attempt_to_steal_funds_using_cross_template_call",
                args![vault_id, attacker, Some(Amount(1000))],
            )
            .sign(test.get_test_secret_key())
            .build(),
        vec![test.get_test_proof()],
    );

    // fails because the function called withdraw on a vault that wasn't in scope. We then check if the vault is owned
    // by the component, but we're not in a component context.
    assert_reject_reason(reason, RuntimeError::NotInComponentContext {
        action: VaultAction::Withdraw.into(),
    });
}

#[test]
fn it_disallows_vault_access_if_vault_is_not_owned() {
    let mut test = TemplateTest::new(["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address("Shenanigans");
    let (victim, _, _) = test.create_owned_account();

    let vault_id = {
        let component = test.read_only_state_store().get_component(victim).unwrap();
        let values = IndexedWellKnownTypes::from_value(component.state()).unwrap();
        values.vault_ids()[0]
    };

    let reason = test.execute_expect_failure(
        Transaction::builder()
            .call_function(template_addr, "ref_stolen_vault", args![vault_id])
            .sign(test.get_test_secret_key())
            .build(),
        vec![test.get_test_proof()],
    );

    // fails because the function called withdraw on a vault that wasnt in scope. We then check if the vault is owned by
    // the component, but we're not in a component context.
    assert_reject_reason(reason, RuntimeError::SubstateOutOfScope {
        address: vault_id.into(),
    });
}
