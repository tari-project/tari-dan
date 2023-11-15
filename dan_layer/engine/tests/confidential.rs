//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress},
    prelude::ConfidentialOutputProof,
};
use tari_template_test_tooling::confidential::{
    generate_confidential_proof, generate_withdraw_proof, generate_withdraw_proof_with_inputs,
};
use tari_template_test_tooling::{SubstateType, TemplateTest};
use tari_transaction_manifest::ManifestValue;

fn setup(
    initial_supply: ConfidentialOutputProof,
) -> (TemplateTest, ComponentAddress, SubstateAddress) {
    let mut template_test = TemplateTest::new(vec![
        "tests/templates/confidential/faucet",
        "tests/templates/confidential/utilities",
    ]);

    let faucet: ComponentAddress =
        template_test.call_function("ConfidentialFaucet", "mint", args![initial_supply], vec![]);

    let resx = template_test.get_previous_output_address(SubstateType::Resource);

    (template_test, faucet, resx)
}

#[test]
fn mint_initial_commitment() {
    let (confidential_proof, _mask, _change) = generate_confidential_proof(Amount(100), None);
    let (mut template_test, faucet, _faucet_resx) = setup(confidential_proof);

    let total_supply: Amount = template_test.call_method(faucet, "total_supply", args![], vec![]);
    // The number of commitments
    assert_eq!(total_supply, Amount(0));
}

#[allow(clippy::too_many_lines)]
#[test]
fn transfer_confidential_amounts_between_accounts() {
    let (confidential_proof, faucet_mask, _change) =
        generate_confidential_proof(Amount(100_000), None);
    let (mut template_test, faucet, faucet_resx) = setup(confidential_proof);

    // Create an account
    let (account1, owner1, _k) = template_test.create_owned_account();
    let (account2, _owner2, _k) = template_test.create_owned_account();

    // Create proof for transfer
    let proof =
        generate_withdraw_proof(&faucet_mask, Amount(1000), Some(Amount(99_000)), Amount(0));

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
    assert_eq!(
        diff.up_iter().filter(|(addr, _)| *addr == account1).count(),
        1
    );
    assert_eq!(
        diff.down_iter()
            .filter(|(addr, _)| *addr == account1)
            .count(),
        1
    );
    assert_eq!(
        diff.up_iter().filter(|(addr, _)| *addr == faucet).count(),
        1
    );
    assert_eq!(
        diff.down_iter().filter(|(addr, _)| *addr == faucet).count(),
        1
    );
    assert_eq!(diff.up_iter().count(), 5);
    assert_eq!(diff.down_iter().count(), 3);

    let withdraw_proof = generate_withdraw_proof(
        &proof.output_mask,
        Amount(100),
        Some(Amount(900)),
        Amount(0),
    );
    let split_proof = generate_withdraw_proof(
        &withdraw_proof.output_mask,
        Amount(20),
        Some(Amount(80)),
        Amount(0),
    );

    let vars = [
        ("faucet_resx", faucet_resx.into()),
        ("account1", account1.into()),
        ("account2", account2.into()),
        (
            "withdraw_proof",
            ManifestValue::new_value(&withdraw_proof.proof).unwrap(),
        ),
        (
            "split_proof",
            ManifestValue::new_value(&split_proof.proof).unwrap(),
        ),
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
        let coins2 = ConfidentialUtilities::split(coins1, split_proof);
        
        account1.deposit(coins1);
        account2.deposit(coins2);
    "#,
            vars,
            vec![owner1],
        )
        .unwrap();
    let diff = result.finalize.result.expect("Failed to execute manifest");
    assert_eq!(
        diff.up_iter().filter(|(addr, _)| *addr == account1).count(),
        1
    );
    assert_eq!(
        diff.down_iter()
            .filter(|(addr, _)| *addr == account1)
            .count(),
        1
    );
    assert_eq!(
        diff.up_iter().filter(|(addr, _)| *addr == account2).count(),
        1
    );
    assert_eq!(
        diff.down_iter()
            .filter(|(addr, _)| *addr == account2)
            .count(),
        1
    );
    assert_eq!(diff.up_iter().count(), 5);
    assert_eq!(diff.down_iter().count(), 3);
}

#[test]
fn transfer_confidential_fails_with_invalid_balance() {
    let (confidential_proof, faucet_mask, _change) =
        generate_confidential_proof(Amount(100_000), None);
    let (mut template_test, faucet, _faucet_resx) = setup(confidential_proof);

    // Create an account
    let (account1, _owner1, _k) = template_test.create_owned_account();

    // Create proof for transfer
    let proof =
        generate_withdraw_proof(&faucet_mask, Amount(1001), Some(Amount(99_000)), Amount(0));

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
    let (confidential_proof, faucet_mask, _change) =
        generate_confidential_proof(Amount(100_000), None);
    let (mut template_test, faucet, faucet_resx) = setup(confidential_proof);

    // Create an account
    let (account1, owner1, _k) = template_test.create_owned_account();
    let (account2, owner2, _k) = template_test.create_owned_account();

    // Create proof for transfer

    let proof =
        generate_withdraw_proof(&faucet_mask, Amount(1000), Some(Amount(99_000)), Amount(0));
    // Reveal 90 tokens and 10 confidentially
    let reveal_proof = generate_withdraw_proof(
        &proof.output_mask,
        Amount(10),
        Some(Amount(900)),
        Amount(90),
    );
    // Then reveal the rest
    let reveal_bucket_proof =
        generate_withdraw_proof(&reveal_proof.output_mask, Amount(0), None, Amount(10));

    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("resource", faucet_resx.into()),
        ("account1", account1.into()),
        ("account2", account2.into()),
        ("proof", ManifestValue::new_value(&proof.proof).unwrap()),
        (
            "reveal_proof",
            ManifestValue::new_value(&reveal_proof.proof).unwrap(),
        ),
        (
            "reveal_bucket_proof",
            ManifestValue::new_value(&reveal_bucket_proof.proof).unwrap(),
        ),
    ];
    let result = template_test
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
        let revealed_funds = account1.reveal_confidential(resource, reveal_proof);
        let revealed_rest_funds = ConfidentialUtilities::reveal(revealed_funds, reveal_bucket_proof);
        account2.deposit(revealed_funds);
        account2.deposit(revealed_rest_funds);
        
        // Account2 can withdraw revealed funds by amount
        let small_amt = account2.withdraw(resource, Amount(10));
        account1.deposit(small_amt);
        
        account1.balance(resource);
        account2.balance(resource);
    "#,
            vars,
            vec![owner1, owner2],
        )
        .unwrap();

    assert_eq!(
        result.finalize.execution_results[12]
            .decode::<Amount>()
            .unwrap(),
        Amount(10)
    );
    assert_eq!(
        result.finalize.execution_results[13]
            .decode::<Amount>()
            .unwrap(),
        Amount(90)
    );
}

#[test]
fn attempt_to_reveal_with_unbalanced_proof() {
    let (confidential_proof, faucet_mask, _change) =
        generate_confidential_proof(Amount(100_000), None);
    let (mut template_test, faucet, faucet_resx) = setup(confidential_proof);

    // Create an account
    let (account1, owner1, _k) = template_test.create_owned_account();
    let (account2, _owner2, _k) = template_test.create_owned_account();

    // Create proof for transfer

    let proof =
        generate_withdraw_proof(&faucet_mask, Amount(1000), Some(Amount(99_000)), Amount(0));
    // Attempt to reveal more than input - change
    let reveal_proof = generate_withdraw_proof(
        &proof.output_mask,
        Amount(0),
        Some(Amount(900)),
        Amount(110),
    );

    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("resource", faucet_resx.into()),
        ("account1", account1.into()),
        ("account2", account2.into()),
        ("proof", ManifestValue::new_value(&proof.proof).unwrap()),
        (
            "reveal_proof",
            ManifestValue::new_value(&reveal_proof.proof).unwrap(),
        ),
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
        let revealed_funds = account1.reveal_confidential(resource, reveal_proof);
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
    let (confidential_proof, faucet_mask, _change) =
        generate_confidential_proof(Amount(100_000), None);
    let (mut template_test, faucet, faucet_resx) = setup(confidential_proof);

    // Create an account
    let (account1, owner1, _k) = template_test.create_owned_account();

    // Create proof for transfer

    let withdraw_proof1 =
        generate_withdraw_proof(&faucet_mask, Amount(1000), Some(Amount(99_000)), Amount(0));
    let withdraw_proof2 = generate_withdraw_proof(
        withdraw_proof1.change_mask.as_ref().unwrap(),
        Amount(1000),
        Some(Amount(98_000)),
        Amount(0),
    );
    let join_proof = generate_withdraw_proof_with_inputs(
        &[
            (withdraw_proof1.output_mask, Amount(1000)),
            (withdraw_proof2.output_mask, Amount(1000)),
        ],
        Amount(2000),
        None,
        Amount(0),
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
        (
            "join_proof",
            ManifestValue::new_value(&join_proof.proof).unwrap(),
        ),
    ];
    let result = template_test
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

    assert_eq!(
        result.finalize.execution_results[3]
            .decode::<u32>()
            .unwrap(),
        1
    );
    assert_eq!(
        result.finalize.execution_results[7]
            .decode::<u32>()
            .unwrap(),
        2
    );
    assert_eq!(
        result.finalize.execution_results[9]
            .decode::<u32>()
            .unwrap(),
        1
    );
}
