//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ToByteType;
use tari_engine_types::{crypto::commit_amount, vault::Vault};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::types::{Amount, ComponentAddress, ResourceType, confidential::ConfidentialOutputStatement};
use tari_template_test_tooling::{
    TemplateTest,
    support::{
        assert_error::assert_reject_reason,
        confidential::{generate_confidential_output_statement, generate_withdraw_proof_with_inputs},
    },
};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn it_does_not_overflow_when_minting_a_huge_initial_supply() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/fungible"]);
    let template = test.get_template_address("Fungible");

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template, "with_supply", args![Amount::MAX])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let component: ComponentAddress = result.finalize.execution_results[0].decode().unwrap();
    let all_to_confidential = generate_withdraw_proof_with_inputs(&[], u64::MAX, u64::MAX, None, 0);
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(component, "convert", args![all_to_confidential.proof])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let confidential_vault = get_confidential_vault(&test, component);
    assert_eq!(confidential_vault.balance(), Amount::MAX - u64::MAX);
    let commitment = confidential_vault
        .get_confidential_commitments()
        .unwrap()
        .iter()
        .next()
        .copied()
        .unwrap();
    let expected = commit_amount(&all_to_confidential.output_mask, u64::MAX.into()).unwrap();
    assert_eq!(commitment, expected.to_byte_type());
    assert_eq!(confidential_vault.get_confidential_commitments().unwrap().len(), 1);
}

#[test]
fn it_does_not_overflow_when_minting_more_then_amount_max_fungible_tokens() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/fungible"]);
    let template = test.get_template_address("Fungible");

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template, "with_supply", args![i64::MAX])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let component: ComponentAddress = result.finalize.execution_results[0].decode().unwrap();

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(component, "fungible_mint_more", args![i64::MAX])
            .call_method(component, "fungible_mint_more", args![i64::MAX])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let vault = get_fungible_vault(&test, component);
    assert_eq!(
        vault.balance(),
        Amount::try_from(i64::MAX).unwrap() * Amount::from(3u64)
    );
}

#[test]
fn it_does_not_overflow_when_minting_more_then_amount_max_confidential_tokens() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/fungible"]);
    let template = test.get_template_address("Fungible");

    let all_to_confidential = generate_withdraw_proof_with_inputs(&[], u64::MAX, u64::MAX, None, 0);
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1)).build_and_seal(test.secret_key()),
        vec![],
    );

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .allocate_component_address("fungible")
            .call_function(template, "with_address_and_supply", args![
                Workspace("fungible"),
                Amount::MAX
            ])
            .call_method("fungible", "convert", args![all_to_confidential.proof])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let component: ComponentAddress = result.finalize.execution_results[1].decode().unwrap();

    let (more_supply1, _mask, _) = generate_confidential_output_statement(u64::MAX, None);
    let (more_supply2, _mask, _) = generate_confidential_output_statement(u64::MAX, None);

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(component, "confidential_mint_more", args![more_supply1])
            .call_method(component, "confidential_mint_more", args![more_supply2])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let confidential_vault = get_confidential_vault(&test, component);
    assert_eq!(confidential_vault.get_confidential_commitments().unwrap().len(), 3);
}

fn get_confidential_vault(test: &TemplateTest, component: ComponentAddress) -> Vault {
    get_vault_by_resource_type(test, component, ResourceType::Confidential)
}

fn get_fungible_vault(test: &TemplateTest, component: ComponentAddress) -> Vault {
    get_vault_by_resource_type(test, component, ResourceType::Fungible)
}

fn get_vault_by_resource_type(test: &TemplateTest, component: ComponentAddress, resource_type: ResourceType) -> Vault {
    let indexed = test
        .read_only_state_store()
        .get_component(component)
        .unwrap()
        .body
        .to_indexed_well_known_types()
        .unwrap();
    indexed
        .vault_ids()
        .iter()
        .find_map(|vault_id| {
            let vault = test.read_only_state_store().get_vault(vault_id).unwrap();
            if vault.resource_type() == resource_type {
                Some(vault)
            } else {
                None
            }
        })
        .expect("No vault found for the specified resource type")
}

/// The confidential resource tracks no supply, so nothing but the vault's own balance bounds a revealed mint.
#[test]
fn minting_past_the_maximum_vault_balance_is_rejected() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/fungible"]);
    let template = test.get_template_address("Fungible");

    let result = test.execute_expect_success(
        test.transaction()
            .call_function(template, "with_supply", args![Amount::MAX])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let component: ComponentAddress = result.finalize.execution_results[0].decode().unwrap();

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_method(component, "confidential_mint_more", args![
                ConfidentialOutputStatement::mint_revealed(1u32)
            ])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "would take the resource balance past the maximum");
}

/// A locked balance is still part of the container's balance, so the maximum applies to the two together.
#[test]
fn minting_past_the_maximum_is_rejected_while_part_of_the_vault_is_locked() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/fungible"]);
    let template = test.get_template_address("Fungible");

    let result = test.execute_expect_success(
        test.transaction()
            .call_function(template, "with_supply", args![Amount::from(100u64)])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let component: ComponentAddress = result.finalize.execution_results[0].decode().unwrap();

    // Locking moves 50 out of the unlocked field, so a mint sized to fit there alone still takes the vault's
    // balance past the maximum once the locked half is counted. The second lock is the operation the bound
    // protects: it adds the unlocked field onto the locked one.
    let reason = test.execute_expect_failure(
        test.transaction()
            .call_method(component, "create_confidential_proof_by_amount", args![Amount::from(
                50u64
            )])
            .put_last_instruction_output_on_workspace("proof")
            .call_method(component, "confidential_mint_more", args![
                ConfidentialOutputStatement::mint_revealed(Amount::MAX - Amount::from(50u64))
            ])
            .call_method(component, "create_confidential_proof_by_amount", args![Amount::MAX])
            .put_last_instruction_output_on_workspace("proof2")
            .drop_all_proofs_in_workspace()
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "would take the resource balance past the maximum");
}

/// A confidential vault carries value as commitments and as a revealed balance, and either alone is enough to
/// take a proof over. `mint_revealed` produces a vault with a revealed balance and no commitments.
#[test]
fn a_proof_over_a_confidential_vault_holding_only_revealed_funds() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/fungible"]);
    let template = test.get_template_address("Fungible");

    let result = test.execute_expect_success(
        test.transaction()
            .call_function(template, "with_supply", args![Amount::from(100u64)])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let component: ComponentAddress = result.finalize.execution_results[0].decode().unwrap();

    let vault = get_confidential_vault(&test, component);
    assert!(vault.get_confidential_commitments().unwrap().is_empty());
    let before = vault.balance();

    test.execute_expect_success(
        test.transaction()
            .call_method(component, "create_confidential_proof", args![])
            .put_last_instruction_output_on_workspace("proof")
            .drop_all_proofs_in_workspace()
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let vault = get_confidential_vault(&test, component);
    assert_eq!(vault.balance(), before);
    assert!(vault.locked_balance().is_zero());
}

/// Taking a proof over a vault and dropping it again moves its balance between the container's locked and
/// unlocked fields and leaves the total where it was.
#[test]
fn a_proof_over_a_confidential_vault_preserves_the_balance() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/fungible"]);
    let template = test.get_template_address("Fungible");

    let result = test.execute_expect_success(
        test.transaction()
            .call_function(template, "with_supply", args![Amount::from(100u64)])
            .build_and_seal(test.secret_key()),
        vec![],
    );
    let component: ComponentAddress = result.finalize.execution_results[0].decode().unwrap();

    // `lock_all` requires a commitment, so move 60 of the 100 revealed into one, leaving 40 revealed.
    let to_commitment = generate_withdraw_proof_with_inputs(&[], 60u64, 60, None, 0u64);
    test.execute_expect_success(
        test.transaction()
            .call_method(component, "convert", args![to_commitment.proof])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let vault = get_confidential_vault(&test, component);
    let before = vault.balance() + vault.locked_balance();

    test.execute_expect_success(
        test.transaction()
            .call_method(component, "create_confidential_proof", args![])
            .put_last_instruction_output_on_workspace("proof")
            .drop_all_proofs_in_workspace()
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let vault = get_confidential_vault(&test, component);
    assert_eq!(vault.balance() + vault.locked_balance(), before);
    assert_eq!(vault.balance(), before);
}
