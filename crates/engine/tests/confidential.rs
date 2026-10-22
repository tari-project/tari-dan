//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::BTreeMap;

use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::PublicKey as _,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_engine::runtime::{ArgumentValidationError, RuntimeError};
use tari_engine_types::{
    crypto::{ElgamalVerifiableBalance, OutputBody, ValueLookup, commit_amount},
    limits::CONFIDENTIAL_LIMITS,
    resource_container::ResourceError,
    substate::SubstateId,
};
use tari_ootle_common_types::substate_type::SubstateType;
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::{
    models::Account,
    types::{
        AccessRule,
        Amount,
        ComponentAddress,
        ConfidentialOutputAddress,
        OwnerRule,
        confidential::{ConfidentialOutputStatement, ConfidentialWithdrawProof},
        constants::TARI_TOKEN,
        crypto::{CommitmentValueProof, PedersenCommitmentBytes, RistrettoPublicKeyBytes},
        metadata,
    },
};
use tari_template_test_tooling::{
    TemplateTest,
    support::{
        GenerateValueLookup,
        assert_error::assert_reject_reason,
        confidential::{
            generate_confidential_output_statement,
            generate_confidential_proof_with_view_key,
            generate_reveal_proof,
            generate_withdraw_proof,
            generate_withdraw_proof_with_inputs,
            generate_withdraw_proof_with_view_key,
        },
        value_proof::{generate_value_proof_mask_knowledge, value_proofs_for_commitment},
    },
};
use tari_transaction_manifest::ManifestValue;
use tari_utilities::ByteArray;

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

/// Generates a mint statement for `amount` along with the mask and the value proofs the engine requires to
/// account for the minted commitment in the resource's total supply.
fn mint_statement(
    amount: u64,
    view_key: Option<&RistrettoPublicKey>,
) -> (ConfidentialOutputStatement, RistrettoSecretKey, ValueProofs) {
    let (statement, mask, _change) = match view_key {
        Some(vk) => generate_confidential_proof_with_view_key(amount, None, vk),
        None => generate_confidential_output_statement(amount, None),
    };
    (statement, mask.clone(), value_proofs_for_commitment(amount, &mask))
}

type ValueProofs = BTreeMap<PedersenCommitmentBytes, CommitmentValueProof>;

fn setup(
    initial_supply: ConfidentialOutputStatement,
    value_proofs: ValueProofs,
    view_key: Option<&RistrettoPublicKey>,
) -> (TemplateTest, ComponentAddress, SubstateId) {
    let mut template_test = TemplateTest::new(CRATE_PATH, vec![
        "tests/templates/confidential/faucet",
        "tests/templates/confidential/utilities",
    ]);

    let faucet: ComponentAddress = match view_key {
        Some(vk) => {
            let vk = RistrettoPublicKeyBytes::from_bytes(vk.as_bytes()).unwrap();
            template_test.call_function(
                "ConfidentialFaucet",
                "mint_with_view_key",
                args![initial_supply, vk, value_proofs],
                vec![],
            )
        },
        None => template_test.call_function(
            "ConfidentialFaucet",
            "mint",
            args![initial_supply, value_proofs],
            vec![],
        ),
    };

    let resx = template_test.get_previous_output_address(SubstateType::Resource);

    (template_test, faucet, resx)
}

#[test]
fn mint_initial_commitment() {
    let (confidential_proof, _mask, value_proofs) = mint_statement(100, None);
    let (test, _faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);

    let resource = test
        .read_only_state_store()
        .get_resource(&faucet_resx.as_resource_address().unwrap())
        .unwrap();
    assert_eq!(resource.total_supply(), Some(Amount::from(100u64)));
}

/// Minting a commitment on a supply-tracking resource is rejected unless the mint proves its value, otherwise the
/// minted value would not be counted towards the supply.
#[test]
fn minting_a_commitment_without_a_value_proof_is_rejected() {
    let (confidential_proof, _mask, value_proofs) = mint_statement(100, None);
    let (mut test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);
    let owner = test.owner_proof();

    let (mint_proof, mint_mask, _value_proofs) = mint_statement(42, None);
    let commitment = commit_amount(&mint_mask, Amount::from(42u64)).unwrap().to_byte_type();

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_method(faucet, "mint_more", args![mint_proof, ValueProofs::new()])
            .build_and_seal(test.secret_key()),
        vec![owner],
    );

    assert_reject_reason(reason, RuntimeError::MissingValueProofForCommitment {
        resource_address: faucet_resx.as_resource_address().unwrap(),
        commitment,
    });
}

#[test]
fn mint_more_later() {
    let (confidential_proof, _mask, value_proofs) = mint_statement(1, None);
    let (mut template_test, faucet, _faucet_resx) = setup(confidential_proof, value_proofs, None);

    let (confidential_proof, mask, value_proofs) = mint_statement(100, None);
    template_test.call_method::<()>(faucet, "mint_more", args![confidential_proof, value_proofs], vec![]);

    let (user_account, user_proof, user_key) = template_test.create_empty_account();

    let withdraw_proof = generate_withdraw_proof(&mask, 100, None, 0u64);
    template_test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "take_free_coins", args![withdraw_proof.proof])
            .put_last_instruction_output_on_workspace("coins")
            .call_method(user_account, "deposit", args![Workspace("coins")])
            .build_and_seal(&user_key),
        vec![user_proof],
    );
}

#[allow(clippy::too_many_lines)]
#[test]
fn transfer_confidential_amounts_between_accounts() {
    let (confidential_proof, faucet_mask, value_proofs) = mint_statement(100_000, None);
    let (mut template_test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);

    // Create an account
    let (account1, owner1, _k) = template_test.create_funded_account();
    let (account2, _owner2, _k) = template_test.create_funded_account();

    // Create proof for transfer
    let proof = generate_withdraw_proof(&faucet_mask, 1000, Some(99_000), 0u64);

    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("account1", account1.into()),
        ("proof", ManifestValue::new_value(&proof.proof).unwrap()),
    ];
    let result = template_test
        .execute_and_commit_manifest(
            r#"
        let faucet = var!["faucet"];
        let account1 = var!["account1"];
        let proof = var!["proof"];
        let coins = faucet.take_free_coins(proof);
        account1.deposit(coins);
    "#,
            vars,
            vec![],
        )
        .unwrap();

    let diff = result.finalize.result.expect("Failed to execute manifest");
    assert_eq!(diff.up_iter().filter(|(addr, _)| *addr == account1).count(), 1);
    assert_eq!(diff.down_iter().filter(|(addr, _)| *addr == account1).count(), 1);
    // Faucet is not changed, only the faucet vault.
    assert_eq!(diff.up_iter().filter(|(addr, _)| *addr == faucet).count(), 0);
    assert_eq!(diff.down_iter().filter(|(addr, _)| *addr == faucet).count(), 0);
    // Up: faucet vault, account1 vault, account1 component, transaction receipt, plus the change and output
    // ConfidentialOutput substates. Down: the two mutated substates plus the spent input ConfidentialOutput.
    assert_eq!(diff.up_iter().count(), 6);
    assert_eq!(diff.down_iter().count(), 3);

    let withdraw_proof = generate_withdraw_proof(&proof.output_mask, 100, Some(900), 0u64);
    let split_proof = generate_withdraw_proof(&withdraw_proof.output_mask, 20, Some(80), 0u64);

    let vars = [
        ("faucet_resx", faucet_resx.into()),
        ("account1", account1.into()),
        ("account2", account2.into()),
        (
            "withdraw_proof",
            ManifestValue::new_value(&withdraw_proof.proof).unwrap(),
        ),
        ("split_proof", ManifestValue::new_value(&split_proof.proof).unwrap()),
    ];
    let result = template_test
        .execute_and_commit_manifest(
            r#"
        let account1 = var!["account1"];
        let account2 = var!["account2"];

        let faucet_resx = var!["faucet_resx"];
        let withdraw_proof = var!["withdraw_proof"];
        let coins1 = account1.withdraw_confidential(faucet_resx, withdraw_proof);

        let split_proof = var!["split_proof"];
        let coins2 = ConfidentialUtilities::take_from_bucket(coins1, split_proof);

        account1.deposit(coins1);
        account2.deposit(coins2);
    "#,
            vars,
            vec![owner1],
        )
        .unwrap();
    let diff = result.finalize.result.expect("Failed to execute manifest");
    assert_eq!(diff.up_iter().filter(|(addr, _)| *addr == account1).count(), 0);
    assert_eq!(diff.down_iter().filter(|(addr, _)| *addr == account1).count(), 0);
    assert_eq!(diff.up_iter().filter(|(addr, _)| *addr == account2).count(), 1);
    assert_eq!(diff.down_iter().filter(|(addr, _)| *addr == account2).count(), 1);
    // Up: account1 vault, account2 vault, account2 component, transaction receipt, plus three materialised
    // ConfidentialOutput substates (withdraw change, split change and split output). The intermediate coins1 output is
    // created and immediately spent by the in-transaction split, so it collapses and never appears in the diff. Down:
    // account1 vault, account2 component and the single spent input ConfidentialOutput.
    assert_eq!(diff.up_iter().count(), 7);
    assert_eq!(diff.down_iter().count(), 3);

    template_test.assert_substate_lengths_match_encoding();
}

#[test]
fn transfer_confidential_fails_with_invalid_balance() {
    let (confidential_proof, faucet_mask, value_proofs) = mint_statement(100_000, None);
    let (mut template_test, faucet, _faucet_resx) = setup(confidential_proof, value_proofs, None);

    // Create an account
    let (account1, _owner1, _k) = template_test.create_funded_account();

    // Create proof for transfer
    let proof = generate_withdraw_proof(&faucet_mask, 1001, Some(99_000), 0u64);

    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("account1", account1.into()),
        ("proof", ManifestValue::new_value(&proof.proof).unwrap()),
    ];
    let _err = template_test
        .execute_and_commit_manifest(
            r#"
        let faucet = var!["faucet"];
        let account1 = var!["account1"];
        let proof = var!["proof"];
        let coins = faucet.take_free_coins(proof);
        account1.deposit(coins);
    "#,
            vars,
            vec![],
        )
        .unwrap_err();
}

#[test]
fn reveal_confidential_and_transfer() {
    let (confidential_proof, faucet_mask, value_proofs) = mint_statement(100_000, None);
    let (mut test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);

    // Create an account
    let (account1, owner1, _k) = test.create_funded_account();
    let (account2, owner2, _k) = test.create_funded_account();

    // Create proof for transfer

    let proof = generate_withdraw_proof(&faucet_mask, 1000, Some(99_000), 0u64);
    // Reveal 90 tokens and 10 confidentially
    let reveal_proof = generate_withdraw_proof(&proof.output_mask, 10, Some(900), 90u64);
    // Then reveal the rest
    let reveal_bucket_proof = generate_withdraw_proof(&reveal_proof.output_mask, 0, None, 10u64);

    let faucet_resx = faucet_resx.as_resource_address().unwrap();
    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("resource", faucet_resx.into()),
        ("account1", account1.into()),
        ("account2", account2.into()),
        ("proof", ManifestValue::new_value(&proof.proof).unwrap()),
        ("reveal_proof", ManifestValue::new_value(&reveal_proof.proof).unwrap()),
        (
            "reveal_bucket_proof",
            ManifestValue::new_value(&reveal_bucket_proof.proof).unwrap(),
        ),
    ];
    let _result = test
        .execute_and_commit_manifest(
            r#"
        let faucet = var!["faucet"];
        let account1 = var!["account1"];
        let account2 = var!["account2"];
        let proof = var!["proof"];
        let reveal_proof = var!["reveal_proof"];
        let reveal_bucket_proof = var!["reveal_bucket_proof"];
        let resource = var!["resource"];

        // Take confidential coins from faucet and deposit into account 1
        let coins = faucet.take_free_coins(proof);
        account1.deposit(coins);

        // Reveal 90 tokens and 10 confidentially and deposit both funds into account 2
        let revealed_funds = account1.withdraw_confidential(resource, reveal_proof);
        let revealed_rest_funds = ConfidentialUtilities::take_from_bucket(revealed_funds, reveal_bucket_proof);
        let joined = ConfidentialUtilities::join_buckets(revealed_funds, revealed_rest_funds);
        account2.deposit(joined);

        // Account2 can withdraw revealed funds by amount
        let small_amt = account2.withdraw(resource, 10);
        account1.deposit(small_amt);

        account1.balance(resource);
        account2.balance(resource);
    "#,
            vars,
            vec![owner1, owner2],
        )
        .unwrap();

    let acc1 = test.read_only_state_store().get_component(account1).unwrap();
    let acc1 = Account::from_value(acc1.state()).unwrap();
    let vault1 = acc1.get_vault_by_resource(&faucet_resx).unwrap();
    let vault1 = test.read_only_state_store().get_vault(&vault1.vault_id()).unwrap();
    assert_eq!(vault1.balance(), 10);

    let acc2 = test.read_only_state_store().get_component(account2).unwrap();
    let acc2 = Account::from_value(acc2.state()).unwrap();
    let vault2 = acc2.get_vault_by_resource(&faucet_resx).unwrap();
    let vault2 = test.read_only_state_store().get_vault(&vault2.vault_id()).unwrap();
    assert_eq!(vault2.balance(), 90);
}

#[test]
fn attempt_to_reveal_with_unbalanced_proof() {
    let (confidential_proof, faucet_mask, value_proofs) = mint_statement(100_000, None);
    let (mut template_test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);

    // Create an account
    let (account1, owner1, _k) = template_test.create_funded_account();
    let (account2, _owner2, _k) = template_test.create_funded_account();

    // Create proof for transfer

    let proof = generate_withdraw_proof(&faucet_mask, 1000, Some(99_000), 0u64);
    // Attempt to reveal more than input - change
    let reveal_proof = generate_withdraw_proof(&proof.output_mask, 0, Some(900), 110u64);

    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("resource", faucet_resx.into()),
        ("account1", account1.into()),
        ("account2", account2.into()),
        ("proof", ManifestValue::new_value(&proof.proof).unwrap()),
        ("reveal_proof", ManifestValue::new_value(&reveal_proof.proof).unwrap()),
    ];

    // TODO: Propagate error messages from runtime
    let _err = template_test
        .execute_and_commit_manifest(
            r#"
        let faucet = var!["faucet"];
        let account1 = var!["account1"];
        let account2 = var!["account2"];
        let proof = var!["proof"];
        let reveal_proof = var!["reveal_proof"];
        let resource = var!["resource"];

        // Take confidential coins from faucet and deposit into account 1
        let coins = faucet.take_free_coins(proof);
        account1.deposit(coins);

        // Reveal 100 tokens and deposit revealed funds into account 2
        let revealed_funds = account1.withdraw_confidential(resource, reveal_proof);
        account2.deposit(revealed_funds);

        account1.balance(resource);
        account2.balance(resource);
    "#,
            vars,
            vec![owner1],
        )
        .unwrap_err();
}

#[test]
fn multi_commitment_join() {
    let (confidential_proof, faucet_mask, value_proofs) = mint_statement(100_000, None);
    let (mut template_test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);

    // Create an account
    let (account1, owner1, _k) = template_test.create_funded_account();

    // Create proof for transfer

    let withdraw_proof1 = generate_withdraw_proof(&faucet_mask, 1000, Some(99_000), 0u64);
    let withdraw_proof2 =
        generate_withdraw_proof(withdraw_proof1.change_mask.as_ref().unwrap(), 1000, Some(98_000), 0u64);
    let join_proof = generate_withdraw_proof_with_inputs(
        &[(withdraw_proof1.output_mask, 1000), (withdraw_proof2.output_mask, 1000)],
        0u64,
        2000,
        None,
        0,
    );

    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("resource", faucet_resx.into()),
        ("account1", account1.into()),
        (
            "withdraw_proof1",
            ManifestValue::new_value(&withdraw_proof1.proof).unwrap(),
        ),
        (
            "withdraw_proof2",
            ManifestValue::new_value(&withdraw_proof2.proof).unwrap(),
        ),
        ("join_proof", ManifestValue::new_value(&join_proof.proof).unwrap()),
    ];
    template_test
        .execute_and_commit_manifest(
            r#"
        let faucet = var!["faucet"];
        let account1 = var!["account1"];
        let withdraw_proof1 = var!["withdraw_proof1"];
        let withdraw_proof2 = var!["withdraw_proof2"];
        let join_proof = var!["join_proof"];
        let resource = var!["resource"];

        // Take confidential coins from faucet and deposit into account
        let coins = faucet.take_free_coins(withdraw_proof1);
        account1.deposit(coins);
        account1.confidential_commitment_count(resource);

        let coins = faucet.take_free_coins(withdraw_proof2);
        account1.deposit(coins);

        // Should contain 2 commitments
        account1.confidential_commitment_count(resource);

        /// Join the two commitments valued at 1000 each
        account1.join_confidential(resource, join_proof);

        // Now we have one commitment valued at 2000
        account1.confidential_commitment_count(resource);
    "#,
            vars,
            vec![owner1],
        )
        .unwrap();

    assert_eq!(template_test.return_values()[3].decoded::<u32>().unwrap(), 1);
    assert_eq!(template_test.return_values()[7].decoded::<u32>().unwrap(), 2);
    assert_eq!(template_test.return_values()[9].decoded::<u32>().unwrap(), 1);
}

#[test]
fn mint_and_transfer_revealed() {
    let (confidential_proof, _mask, value_proofs) = mint_statement(100, None);
    let (mut test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);

    let faucet_resx = faucet_resx.as_resource_address().unwrap();

    let (user_account, _, _) = test.create_empty_account();

    test.call_method::<()>(faucet, "mint_revealed", args![123], vec![]);
    let balance: Amount = test.call_method(faucet, "vault_balance", args![], vec![]);
    assert_eq!(balance, Amount::from(123u64));

    // Convert 100 revealed funds to confidential and the remaining 23 to revealed
    let withdraw = generate_withdraw_proof_with_inputs(&[], 123u64, 100, None, 23);

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "take_free_coins", args![withdraw.proof])
            .put_last_instruction_output_on_workspace("b")
            .call_method(user_account, "deposit", args![Workspace("b")])
            .call_method(user_account, "balance", args![faucet_resx])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    // The account should have a revealed balance of 23 revealed funds
    let account_balance = test.return_values()[3].decoded::<Amount>().unwrap();
    assert_eq!(account_balance, 23);
}

#[test]
fn mint_revealed_with_invalid_proof() {
    let (confidential_proof, _mask, value_proofs) = mint_statement(100, None);
    let (mut test, faucet, _faucet_resx) = setup(confidential_proof, value_proofs, None);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "mint_revealed_with_bad_range_proof", args![123])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, ResourceError::InvalidConfidentialProof {
        details: String::new(),
    });
}

pub fn try_brute_force_confidential_balance<L>(
    outputs: &[OutputBody],
    secret_view_key: &RistrettoSecretKey,
    value_lookup: &L,
) -> Result<Option<u64>, L::Error>
where
    L: ValueLookup,
{
    let decompressed_viewable_balances = outputs
        .iter()
        .filter_map(|output| output.viewable_balance.as_ref().map(|vb| vb.try_into().unwrap()))
        .collect::<Vec<_>>();

    let balances =
        ElgamalVerifiableBalance::decrypt_many(secret_view_key, &decompressed_viewable_balances, value_lookup)?;

    // If any of the commitments cannot be decrypted, then we return None
    Ok(balances.into_iter().sum())
}

#[test]
fn mint_with_view_key() {
    let (view_key_secret, ref view_key) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    let (confidential_proof, _mask, value_proofs) = mint_statement(123, Some(view_key));
    let (mut test, faucet, _faucet_resx) = setup(confidential_proof, value_proofs, Some(view_key));

    let (confidential_proof, mask, value_proofs) = mint_statement(100, Some(view_key));
    test.call_method::<()>(faucet, "mint_more", args![confidential_proof, value_proofs], vec![]);

    let (user_account, user_proof, user_key) = test.create_empty_account();

    let withdraw_proof = generate_withdraw_proof_with_view_key(&mask, 100, 55, Some(100 - 55), 0u64, view_key);
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "take_free_coins", args![withdraw_proof.proof])
            .put_last_instruction_output_on_workspace("coins")
            .call_method(user_account, "deposit", args![Workspace("coins")])
            .build_and_seal(&user_key),
        vec![user_proof],
    );

    let (_, faucet_vault) = test
        .read_only_state_store()
        .get_vaults_for_component(faucet)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();

    let faucet_outputs = test
        .read_only_state_store()
        .get_confidential_output_bodies(&faucet_vault)
        .unwrap();
    let total_balance =
        try_brute_force_confidential_balance(&faucet_outputs, &view_key_secret, &GenerateValueLookup::new(0..=200))
            .unwrap();
    assert_eq!(total_balance, Some(223 - 55));

    let (_, user_vault) = test
        .read_only_state_store()
        .get_vaults_for_component(user_account)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();

    let user_outputs = test
        .read_only_state_store()
        .get_confidential_output_bodies(&user_vault)
        .unwrap();
    let total_balance =
        try_brute_force_confidential_balance(&user_outputs, &view_key_secret, &GenerateValueLookup::new(0..=200))
            .unwrap();
    assert_eq!(total_balance, Some(55));
}

#[test]
fn freeze_then_attempt_spend() {
    let (confidential_proof, mask, value_proofs) = mint_statement(100, None);
    let (mut test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);

    let commitment = commit_amount(&mask, Amount::from(100u64)).unwrap().to_byte_type();
    let frozen_address = ConfidentialOutputAddress::new(faucet_resx.as_resource_address().unwrap(), commitment);

    let owner = test.owner_proof();
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "freeze_confidential_outputs", args![vec![commitment]])
            .build_and_seal(test.secret_key()),
        vec![owner.clone()],
    );

    let (user_account, user_proof, user_key) = test.create_empty_account();
    let withdraw_proof = generate_withdraw_proof(&mask, 100, None, 0u64);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "take_free_coins", args![withdraw_proof.proof.clone()])
            .put_last_instruction_output_on_workspace("coins")
            .call_method(user_account, "deposit", args![Workspace("coins")])
            .build_and_seal(&user_key),
        vec![user_proof.clone()],
    );

    assert_reject_reason(reason, ResourceError::InvalidSpend {
        details: format!("Confidential output {frozen_address} is frozen"),
    });

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "unfreeze_confidential_outputs", args![vec![commitment]])
            .build_and_seal(test.secret_key()),
        vec![owner],
    );

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "take_free_coins", args![withdraw_proof.proof])
            .put_last_instruction_output_on_workspace("coins")
            .call_method(user_account, "deposit", args![Workspace("coins")])
            .build_and_seal(&user_key),
        vec![user_proof],
    );
}

#[test]
fn freezing_confidential_outputs_invokes_the_auth_hook() {
    let (confidential_proof, mask, value_proofs) = mint_statement(100, None);
    let mut test = TemplateTest::new(CRATE_PATH, vec!["tests/templates/confidential/faucet"]);
    let faucet: ComponentAddress = test.call_function(
        "ConfidentialFaucet",
        "mint_with_auth_hook",
        args![confidential_proof, value_proofs],
        vec![],
    );
    let faucet_resx = test
        .get_previous_output_address(SubstateType::Resource)
        .as_resource_address()
        .unwrap();
    let commitment = commit_amount(&mask, Amount::from(100u64)).unwrap().to_byte_type();

    let owner = test.owner_proof();
    test.call_function::<()>(
        "ConfidentialFaucet",
        "freeze_confidential_outputs_of",
        args![faucet_resx, vec![commitment]],
        vec![owner],
    );

    let hook_calls: u64 = test.call_method(faucet, "hook_calls", args![], vec![]);
    assert_eq!(hook_calls, 1);
}

/// A pre-existing output that is unfrozen and spent by the same transaction must still be downed. The unfreeze
/// mutates it, which moves it to the same place the engine holds newly-created substates, and a spend must not
/// mistake that for "created in this transaction" and collapse it: that would leave it live in global state
/// with its value already spent.
#[test]
fn unfreeze_and_spend_in_one_transaction_downs_the_output() {
    let (confidential_proof, mask, value_proofs) = mint_statement(100, None);
    let (mut test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);

    let commitment = commit_amount(&mask, Amount::from(100u64)).unwrap().to_byte_type();
    let output_address = ConfidentialOutputAddress::new(faucet_resx.as_resource_address().unwrap(), commitment);

    let owner = test.owner_proof();
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "freeze_confidential_outputs", args![vec![commitment]])
            .build_and_seal(test.secret_key()),
        vec![owner.clone()],
    );

    let (user_account, _user_proof, _user_key) = test.create_empty_account();
    let withdraw_proof = generate_withdraw_proof(&mask, 100, None, 0u64);

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "unfreeze_confidential_outputs", args![vec![commitment]])
            .call_method(faucet, "take_free_coins", args![withdraw_proof.proof])
            .put_last_instruction_output_on_workspace("coins")
            .call_method(user_account, "deposit", args![Workspace("coins")])
            .build_and_seal(test.secret_key()),
        vec![owner],
    );

    let diff = result.finalize.any_accept().unwrap();
    let spent_id = SubstateId::ConfidentialOutput(output_address);
    assert!(
        diff.down_iter().any(|(id, _)| *id == spent_id),
        "spent output was not downed: it stays live in global state. Downs: {:?}",
        diff.down_iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>()
    );
    assert!(
        !diff.up_iter().any(|(id, _)| *id == spent_id),
        "spent output must not be upped again"
    );
}

/// The receipt records a spent output, which is downed and never upped again, and leaves out the downs of
/// substates it upped at a later version.
#[test]
fn receipt_lists_the_spent_output_as_downed() {
    let (confidential_proof, mask, value_proofs) = mint_statement(100, None);
    let (mut test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);

    let commitment = commit_amount(&mask, Amount::from(100u64)).unwrap().to_byte_type();
    let output_address = ConfidentialOutputAddress::new(faucet_resx.as_resource_address().unwrap(), commitment);

    let (user_account, user_proof, user_key) = test.create_empty_account();
    let withdraw_proof = generate_withdraw_proof(&mask, 100, None, 0u64);

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "take_free_coins", args![withdraw_proof.proof])
            .put_last_instruction_output_on_workspace("coins")
            .call_method(user_account, "deposit", args![Workspace("coins")])
            .build_and_seal(&user_key),
        vec![user_proof],
    );

    let diff = result.finalize.any_accept().unwrap();
    let receipt = diff
        .up_iter()
        .find_map(|(_, substate)| substate.substate_value().as_transaction_receipt())
        .expect("the diff ups the transaction receipt");
    let downed = receipt
        .diff_summary()
        .downed
        .iter()
        .map(|d| &d.substate_id)
        .collect::<Vec<_>>();

    assert_eq!(downed, [&SubstateId::ConfidentialOutput(output_address)]);
    assert!(
        diff.down_iter()
            .any(|(id, _)| diff.up_iter().any(|(up_id, _)| up_id == id)),
        "the transaction must also rewrite a substate for the exclusion to be exercised"
    );
}

/// The commitment is the output's identity: minting the same commitment twice must be rejected by the
/// duplicate-substate guard, which is what provides network-wide commitment uniqueness now that outputs are
/// no longer keyed by a per-vault map.
#[test]
fn minting_a_duplicate_commitment_is_rejected() {
    let (confidential_proof, _mask, value_proofs) = mint_statement(100, None);
    let (mut test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);

    let (mint_proof, mint_mask, mint_value_proofs) = mint_statement(42, None);
    let owner = test.owner_proof();
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "mint_more", args![
                mint_proof.clone(),
                mint_value_proofs.clone()
            ])
            .build_and_seal(test.secret_key()),
        vec![owner.clone()],
    );

    let commitment = commit_amount(&mint_mask, Amount::from(42u64)).unwrap().to_byte_type();
    let address = ConfidentialOutputAddress::new(faucet_resx.as_resource_address().unwrap(), commitment);
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "mint_more", args![mint_proof, mint_value_proofs])
            .build_and_seal(test.secret_key()),
        vec![owner],
    );

    assert_reject_reason(reason, RuntimeError::DuplicateSubstate {
        address: address.into(),
    });
}

/// The confidential-withdraw caps must be enforced on the withdraw execution path, not only exist as
/// standalone checks: one withdraw over `max_withdraws_per_transaction` rejects the whole transaction.
#[test]
fn a_transaction_over_the_withdraw_cap_is_rejected() {
    let (confidential_proof, _mask, value_proofs) = mint_statement(100, None);
    let (mut test, faucet, _faucet_resx) = setup(confidential_proof, value_proofs, None);
    let owner = test.owner_proof();

    let max_withdraws = CONFIDENTIAL_LIMITS.max_withdraws_per_transaction;
    // Revealed funds make each withdraw a revealed-only confidential withdraw: no commitments are needed
    // and the per-withdraw work is trivial, so only the per-transaction cap is exercised.
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "mint_revealed", args![Amount::new(max_withdraws as u128 + 1)])
            .build_and_seal(test.secret_key()),
        vec![owner.clone()],
    );

    let mut builder = Transaction::builder_localnet(Epoch(1));
    for _ in 0..=max_withdraws {
        builder = builder.call_method(faucet, "take_free_coins", args![
            ConfidentialWithdrawProof::revealed_withdraw(Amount::new(1))
        ]);
    }
    let reason = test.execute_expect_failure(builder.build_and_seal(test.secret_key()), vec![owner]);

    assert_reject_reason(
        reason,
        ArgumentValidationError::MaxConfidentialWithdrawsPerTransactionExceeded { max_withdraws },
    );
}

fn read_total_supply(test: &TemplateTest, resource: &SubstateId) -> Option<Amount> {
    test.read_only_state_store()
        .get_resource(&resource.as_resource_address().unwrap())
        .unwrap()
        .total_supply()
}

/// Value minted into a commitment counts towards `total_supply`, revealing it is supply-neutral, and burning
/// the revealed bucket removes it again.
#[test]
fn minting_and_burning_a_commitment_tracks_total_supply() {
    let (confidential_proof, mask, value_proofs) = mint_statement(500, None);
    let (mut test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);
    let owner = test.owner_proof();
    let utilities = test.get_template_address("ConfidentialUtilities");

    assert_eq!(read_total_supply(&test, &faucet_resx), Some(Amount::from(500u64)));

    // Move the whole 500 out of the commitment and into revealed funds.
    let reveal_proof = generate_reveal_proof(&mask, 500);
    test.execute_expect_success(
        test.transaction()
            .call_method(faucet, "take_free_coins", args![reveal_proof])
            .put_last_instruction_output_on_workspace("coins")
            .call_function(utilities, "burn_bucket", args![Workspace("coins")])
            .build_and_seal(test.secret_key()),
        vec![owner],
    );

    assert_eq!(read_total_supply(&test, &faucet_resx), Some(Amount::ZERO));
}

/// A bucket still holding commitments has an amount the engine cannot see, so burning it while supply
/// tracking is enabled requires a value proof for each commitment.
#[test]
fn burning_a_bucket_holding_commitments_without_a_value_proof_is_rejected() {
    let (confidential_proof, mask, value_proofs) = mint_statement(500, None);
    let (mut test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);
    let owner = test.owner_proof();
    let utilities = test.get_template_address("ConfidentialUtilities");

    let withdraw_proof = generate_withdraw_proof(&mask, 500, None, 0u64);
    let commitment = commit_amount(&withdraw_proof.output_mask, Amount::from(500u64))
        .unwrap()
        .to_byte_type();
    let reason = test.execute_expect_failure(
        test.transaction()
            .call_method(faucet, "take_free_coins", args![withdraw_proof.proof])
            .put_last_instruction_output_on_workspace("coins")
            .call_function(utilities, "burn_bucket", args![Workspace("coins")])
            .build_and_seal(test.secret_key()),
        vec![owner],
    );

    assert_reject_reason(reason, RuntimeError::MissingValueProofForCommitment {
        resource_address: faucet_resx.as_resource_address().unwrap(),
        commitment,
    });
}

/// Given a value proof per commitment, a bucket holding commitments burns and the resource's total supply drops
/// by the value destroyed. This is the path a holder who knows the masks takes instead of revealing first.
#[test]
fn burning_a_bucket_holding_commitments_with_value_proofs_decreases_total_supply() {
    let (confidential_proof, mask, value_proofs) = mint_statement(500, None);
    let (mut test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);
    let owner = test.owner_proof();
    let utilities = test.get_template_address("ConfidentialUtilities");

    assert_eq!(read_total_supply(&test, &faucet_resx), Some(Amount::from(500u64)));

    // Split the 500 into a 200 output and 300 change, then burn the 200 output commitment.
    let withdraw_proof = generate_withdraw_proof(&mask, 200, Some(300), 0u64);
    let burnt = Amount::from(200u64);
    let commitment = commit_amount(&withdraw_proof.output_mask, burnt)
        .unwrap()
        .to_byte_type();
    let value_proofs = BTreeMap::from([(
        commitment,
        generate_value_proof_mask_knowledge(burnt, &withdraw_proof.output_mask),
    )]);

    test.execute_expect_success(
        test.transaction()
            .call_method(faucet, "take_free_coins", args![withdraw_proof.proof])
            .put_last_instruction_output_on_workspace("coins")
            .call_function(utilities, "burn_bucket_with_value_proofs", args![
                Workspace("coins"),
                value_proofs
            ])
            .build_and_seal(test.secret_key()),
        vec![owner],
    );

    assert_eq!(read_total_supply(&test, &faucet_resx), Some(Amount::from(300u64)));
}

/// The liquidity pool prices and pays out only revealed funds, so it refuses a confidential bucket that carries
/// commitments, while still accepting one that is fully revealed.
#[test]
fn liquidity_pool_rejects_confidential_commitments() {
    let (confidential_proof, mask, value_proofs) = mint_statement(100_000, None);
    let (mut test, faucet, faucet_resx) = setup(confidential_proof, value_proofs, None);
    let faucet_resx = faucet_resx.as_resource_address().unwrap();
    let pool_template = test.get_template_address("TwoResourceLiquidityPool");
    let (owner, owner_proof, owner_secret) = test.create_funded_account();

    test.call_method::<()>(faucet, "mint_revealed", args![20_000], vec![]);

    let revealed = ConfidentialWithdrawProof::revealed_withdraw::<u64>;

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .allocate_component_address("pool")
            .call_function(pool_template, "create", args![
                OwnerRule::OwnedBySigner,
                AccessRule::AllowAll,
                TARI_TOKEN,
                faucet_resx,
                metadata!["name" => "TARI-Confidential Liquidity Pool"],
                Workspace("pool"),
            ])
            .call_method(faucet, "take_free_coins", args![revealed(10_000)])
            .put_last_instruction_output_on_workspace("conf_coins")
            .call_method(owner, "withdraw", args![TARI_TOKEN, 10_000])
            .put_last_instruction_output_on_workspace("tari_coins")
            .call_method("pool", "contribute", args![
                Workspace("tari_coins"),
                Workspace("conf_coins")
            ])
            .put_last_instruction_output_on_workspace("contribution")
            .call_method(owner, "deposit", args![Workspace("contribution.0")])
            .call_method(owner, "deposit", args![Workspace("contribution.1")])
            .call_method(owner, "deposit", args![Workspace("contribution.2")])
            .build_and_seal(&owner_secret),
        vec![owner_proof.clone()],
    );

    let (pool, _) = test
        .read_only_state_store()
        .get_components_by_template_address(pool_template)
        .unwrap()
        .pop()
        .unwrap();

    // A 50,000 commitment alongside 1,000 revealed
    let mixed = || generate_withdraw_proof(&mask, 50_000, Some(49_000), 1_000u64).proof;

    for method in ["swap", "swap_constant_product"] {
        let reason = test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(faucet, "take_free_coins", args![mixed()])
                .put_last_instruction_output_on_workspace("input")
                .call_method(pool, method, args![Workspace("input")])
                .put_last_instruction_output_on_workspace("output")
                .call_method(owner, "deposit", args![Workspace("output")])
                .build_and_seal(&owner_secret),
            vec![owner_proof.clone()],
        );
        assert_reject_reason(&reason, "Expected bucket to have no confidential commitments");
    }

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "take_free_coins", args![mixed()])
            .put_last_instruction_output_on_workspace("input")
            .call_method(pool, "protected_add_liquidity", args![Workspace("input")])
            .build_and_seal(&owner_secret),
        vec![owner_proof.clone()],
    );
    assert_reject_reason(&reason, "Expected bucket to have no confidential commitments");

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "take_free_coins", args![revealed(1_000)])
            .put_last_instruction_output_on_workspace("input")
            .call_method(pool, "swap", args![Workspace("input")])
            .put_last_instruction_output_on_workspace("output")
            .call_method(owner, "deposit", args![Workspace("output")])
            .call_method(faucet, "take_free_coins", args![revealed(1_000)])
            .put_last_instruction_output_on_workspace("added")
            .call_method(pool, "protected_add_liquidity", args![Workspace("added")])
            .build_and_seal(&owner_secret),
        vec![owner_proof],
    );
}
