//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine::runtime::{ActionIdent, NativeAction, RuntimeError};
use tari_engine_types::{
    commit_result::RejectReason,
    indexed_value::IndexedWellKnownTypes,
    resource_container::ResourceError,
};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::{
    args::VaultAction,
    types::{Amount, ComponentAddress, ResourceType, constants::TARI_TOKEN},
};
use tari_template_test_tooling::{TemplateTest, support::assert_error::assert_reject_reason};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");
const TEMPLATE_NAME: &str = "Shenanigans";

#[test]
fn it_rejects_dangling_vaults_in_constructor() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "dangling_vault", args![])
            .build_and_seal(test.secret_key()),
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
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "return_vault", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "1 orphaned substate(s) detected");
}

#[test]
fn it_rejects_dangling_vaults_in_component() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    //  Create with vault
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "with_vault", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let component_address = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();
    let component = test.read_only_state_store().get_component(component_address).unwrap();
    let indexed = IndexedWellKnownTypes::from_value(component.state()).unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(component_address, "drop_vault", args![])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    assert_reject_reason(reason, RuntimeError::OrphanedSubstate {
        id: indexed.vault_ids()[0].into(),
    });
}

#[test]
fn it_rejects_dangling_resources() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "dangling_resource", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "1 orphaned substate(s) detected")
}

#[test]
fn it_rejects_unknown_substate_ids() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "non_existent_id", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(
        reason,
        RejectReason::SubstateNotFound(
            "At instruction #1: Template referenced substate but it was not found: \
             resource_abababababababababababababababababababababababababababababababab"
                .to_string(),
        ),
    )
}

#[test]
fn it_rejects_references_to_buckets_that_arent_in_scope() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let (account, owner_token, owner_key) = test.create_funded_account();

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "with_vault", args![])
            .build_and_seal(&owner_key),
        vec![owner_token.clone()],
    );

    let shenanigans = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(account, "withdraw", args![TARI_TOKEN, 1000])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(shenanigans, "take_bucket_zero", args![])
            .build_and_seal(&owner_key),
        vec![owner_token],
    );

    // take_bucket_zero fails because the bucket isnt in scope for the call
    assert_reject_reason(reason, RuntimeError::BucketNotFound { bucket_id: 0.into() });
}

#[test]
fn it_rejects_double_ownership_of_vault() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "with_vault_copy", args![])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    assert_reject_reason(reason, "Duplicate reference to substate");
}

#[test]
fn it_prevents_access_to_vault_id_in_component_context() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let (account, _, _) = test.create_funded_account();

    let vault_id = {
        let component = test.read_only_state_store().get_component(account).unwrap();
        let values = IndexedWellKnownTypes::from_value(component.state()).unwrap();
        values.vault_ids()[0]
    };

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "with_vault", args![])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    let shenanigans = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(shenanigans, "take_from_a_vault", args![vault_id, 1000])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    // take_bucket_zero fails because the component didnt create the vault
    assert_reject_reason(reason, RuntimeError::SubstateNotOwned {
        id: vault_id.into(),
        requested_owner: Box::new(shenanigans.into()),
    });
}

#[test]
fn it_prevents_access_to_out_of_scope_component() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let (account, _, _) = test.create_funded_account();

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "new", args![])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    let shenanigans = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(shenanigans, "empty_state_on_component", args![account])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    // Fails because the engine does not lock this component
    assert_reject_reason(reason, RuntimeError::AccessDeniedSetComponentState {
        attempted_on: account.into(),
        attempted_by: Box::new(shenanigans.into()),
    });
}

#[test]
fn it_disallows_calls_on_vaults_that_are_not_owned_by_current_component() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let (victim, _, _) = test.create_funded_account();
    let (attacker, _, _) = test.create_empty_account();

    let vault_id = {
        let component = test.read_only_state_store().get_component(victim).unwrap();
        let values = IndexedWellKnownTypes::from_value(component.state()).unwrap();
        values.vault_ids()[0]
    };

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(
                template_addr,
                "attempt_to_steal_funds_using_cross_template_call",
                args![vault_id, attacker, Some(Amount::from(1000u64))],
            )
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    // fails because the function called withdraw on a vault that wasn't in scope. We then check if the vault is owned
    // by the component, but we're not in a component context.
    assert_reject_reason(reason, RuntimeError::NotInComponentContext {
        action: VaultAction::Withdraw.into(),
    });
}

#[test]
fn it_disallows_vault_access_if_vault_is_not_owned() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let (victim, _, _) = test.create_funded_account();

    let vault_id = {
        let component = test.read_only_state_store().get_component(victim).unwrap();
        let values = IndexedWellKnownTypes::from_value(component.state()).unwrap();
        values.vault_ids()[0]
    };

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "ref_stolen_vault", args![vault_id])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    // fails because the function called withdraw on a vault that wasnt in scope. We then check if the vault is owned by
    // the component, but we're not in a component context.
    assert_reject_reason(reason, RuntimeError::SubstateOutOfScope { id: vault_id.into() });
}

#[test]
fn it_disallows_minting_different_resource_type() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/resource_shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let (account, _, _) = test.create_empty_account();

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "new", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let component = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(component, "mint_different_resource_type", args![])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(account, "deposit", args![Workspace("bucket")])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    // We explicitly check that the mint fails. The deposit will also fail with a resource type mismatch, but if that
    // happened, it means we were able to create a bucket in the first place, which should not be permitted.
    assert_reject_reason(reason, ResourceError::ResourceTypeMismatch {
        operate: "mint",
        expected: ResourceType::NonFungible,
        given: ResourceType::Fungible,
    });
}

#[test]
fn it_does_not_bring_non_owned_vault_id_into_scope() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let (account, _, _) = test.create_funded_account();
    let vault_id = {
        let store = test.read_only_state_store();
        let component = store.get_component(account).unwrap();
        let values = IndexedWellKnownTypes::from_value(component.state()).unwrap();
        values.vault_ids()[0]
    };

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "with_stolen_vault", args![vault_id])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(account, "deposit", args![Workspace("bucket")])
            .add_input(vault_id)
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, RuntimeError::SubstateOutOfScope { id: vault_id.into() });
}

#[test]
fn it_disallows_withdraws_from_vaults_outside_of_component_context() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let (account, _, _) = test.create_funded_account();
    let vault_id = {
        let store = test.read_only_state_store();
        let component = store.get_component(account).unwrap();
        let values = IndexedWellKnownTypes::from_value(component.state()).unwrap();
        values.vault_ids()[0]
    };

    let template_addr = test.compile_new_template(TEMPLATE_NAME, "tests/templates/shenanigans", &[], [(
        "VAULT_ID",
        vault_id.to_string(),
    )]);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "take_from_hardcoded_vault", args![])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(account, "deposit", args![Workspace("bucket")])
            .add_input(vault_id)
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, RuntimeError::NotInComponentContext {
        action: ActionIdent::Native(NativeAction::Vault(VaultAction::Withdraw)),
    });

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "take_from_vault_and_return_bucket", args![vault_id,])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(account, "deposit", args![Workspace("bucket")])
            .add_input(vault_id)
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, RuntimeError::NotInComponentContext {
        action: ActionIdent::Native(NativeAction::Vault(VaultAction::Withdraw)),
    });
}

#[test]
fn it_disallows_withdraws_from_vaults_outside_of_owning_component() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let (account, _, _) = test.create_funded_account();
    let vault_id = {
        let store = test.read_only_state_store();
        let component = store.get_component(account).unwrap();
        let values = IndexedWellKnownTypes::from_value(component.state()).unwrap();
        values.vault_ids()[0]
    };

    // Compile a template with the vault id hardcoded
    let template_addr = test.compile_new_template(TEMPLATE_NAME, "tests/templates/shenanigans", &[], [(
        "VAULT_ID",
        vault_id.to_string(),
    )]);

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "new", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let component = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(component, "take_from_hardcoded_vault_in_component_context", args![])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(account, "deposit", args![Workspace("bucket")])
            .add_input(vault_id)
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    assert_reject_reason(reason, RuntimeError::SubstateNotOwned {
        id: vault_id.into(),
        requested_owner: Box::new(component.into()),
    });
}

#[test]
fn it_does_not_leak_a_callees_vaults_into_the_callers_scope() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let (victim, _, _) = test.create_funded_account();
    let (attacker, _, _) = test.create_empty_account();

    let vault_id = {
        let store = test.read_only_state_store();
        let component = store.get_component(victim).unwrap();
        let values = IndexedWellKnownTypes::from_value(component.state()).unwrap();
        values.vault_ids()[0]
    };

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_function(template_addr, "call_then_steal_from_vault", args![victim, vault_id])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(attacker, "deposit", args![Workspace("bucket")])
            .add_input(vault_id)
            .build_and_seal(test.secret_key()),
        vec![],
    );

    // Calling the victim leaves the attacker's frame without a component context, so the vault is neither in scope
    // nor owned by a component the attacker is executing on.
    assert_reject_reason(reason, RuntimeError::NotInComponentContext {
        action: VaultAction::Withdraw.into(),
    });
}

#[test]
fn it_rejects_a_bucket_that_is_neither_consumed_nor_returned_by_a_call() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    let result = test.execute_expect_success(
        test.transaction()
            .call_function(template_addr, "with_fungible_vault", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let component = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_method(component, "abandon_bucket", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "were neither consumed nor returned by the call");
}

#[test]
fn it_rejects_a_proof_that_is_neither_dropped_nor_returned_by_a_call() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    let result = test.execute_expect_success(
        test.transaction()
            .call_function(template_addr, "with_fungible_vault", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let component = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_method(component, "abandon_proof", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "were neither dropped nor returned by the call");
}

#[test]
fn a_proof_outlives_the_authorization_taken_from_it() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    let result = test.execute_expect_success(
        test.transaction()
            .call_function(template_addr, "with_fungible_vault", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let component = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    test.execute_expect_success(
        test.transaction()
            .call_method(component, "authorize_then_drop_proof", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    test.execute_expect_success(
        test.transaction()
            .call_method(component, "authorize_then_return_proof", args![])
            .put_last_instruction_output_on_workspace("proof")
            .drop_all_proofs_in_workspace()
            .build_and_seal(test.secret_key()),
        vec![],
    );
}

#[test]
fn it_does_not_leak_a_callees_vaults_into_a_calling_components_scope() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);
    let (victim, _, _) = test.create_funded_account();
    let (attacker_account, _, _) = test.create_empty_account();

    let result = test.execute_expect_success(
        test.transaction()
            .call_function(template_addr, "with_fungible_vault", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let attacker = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    let vault_id = {
        let store = test.read_only_state_store();
        let component = store.get_component(victim).unwrap();
        let values = IndexedWellKnownTypes::from_value(component.state()).unwrap();
        values.vault_ids()[0]
    };

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_method(attacker, "call_then_steal_from_vault_as_component", args![
                victim, vault_id
            ])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(attacker_account, "deposit", args![Workspace("bucket")])
            .add_input(vault_id)
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, RuntimeError::SubstateNotOwned {
        id: vault_id.into(),
        requested_owner: Box::new(attacker.into()),
    });
}

#[test]
fn it_does_not_leak_a_callees_address_allocation_into_the_callers_scope() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    let result = test.execute_expect_success(
        test.transaction()
            .call_function(template_addr, "with_fungible_vault", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let victim = result.finalize.execution_results[0]
        .decode::<ComponentAddress>()
        .unwrap();

    // An allocation is a reservation of an address, and nothing checks that the template consuming one is the
    // template that made it, so the caller must never be handed one it was not given.
    let reason = test.execute_expect_failure(
        test.transaction()
            .call_function(template_addr, "call_then_use_abandoned_allocation", args![victim, 0u32])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, RuntimeError::AddressAllocationNotInScope { id: 0 });
}
