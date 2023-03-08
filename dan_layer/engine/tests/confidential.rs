//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::encode;
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress},
    prelude::ConfidentialOutputProof,
};
use tari_template_test_tooling::{SubstateType, TemplateTest};
use tari_transaction_manifest::ManifestValue;

use self::utilities::*;

fn setup(initial_supply: ConfidentialOutputProof) -> (TemplateTest, ComponentAddress, SubstateAddress) {
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

#[test]
fn transfer_confidential_amounts_between_accounts() {
    let (confidential_proof, faucet_mask, _change) = generate_confidential_proof(Amount(100_000), None);
    let (mut template_test, faucet, faucet_resx) = setup(confidential_proof);

    // Create an account
    let (account1, owner1, _k) = template_test.create_owned_account();
    let (account2, _owner2, _k) = template_test.create_owned_account();

    // Create proof for transfer
    let proof = generate_withdraw_proof(&faucet_mask, Amount(1000), Some(Amount(99_000)), Amount(0));

    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("account1", account1.into()),
        ("proof", ManifestValue::Value(encode(&proof.proof).unwrap())),
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
    let diff = result.result.expect("Failed to execute manifest");
    assert_eq!(diff.up_iter().filter(|(addr, _)| *addr == account1).count(), 1);
    assert_eq!(diff.down_iter().filter(|(addr, _)| *addr == account1).count(), 1);
    assert_eq!(diff.up_iter().filter(|(addr, _)| *addr == faucet).count(), 1);
    assert_eq!(diff.down_iter().filter(|(addr, _)| *addr == faucet).count(), 1);
    assert_eq!(diff.up_iter().count(), 4);
    assert_eq!(diff.down_iter().count(), 3);

    let withdraw_proof = generate_withdraw_proof(&proof.output_mask, Amount(100), Some(Amount(900)), Amount(0));
    let split_proof = generate_withdraw_proof(&withdraw_proof.output_mask, Amount(20), Some(Amount(80)), Amount(0));

    let vars = [
        ("faucet_resx", faucet_resx.into()),
        ("account1", account1.into()),
        ("account2", account2.into()),
        (
            "withdraw_proof",
            ManifestValue::Value(encode(&withdraw_proof.proof).unwrap()),
        ),
        ("split_proof", ManifestValue::Value(encode(&split_proof.proof).unwrap())),
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
    let diff = result.result.expect("Failed to execute manifest");
    assert_eq!(diff.up_iter().filter(|(addr, _)| *addr == account1).count(), 1);
    assert_eq!(diff.down_iter().filter(|(addr, _)| *addr == account1).count(), 1);
    assert_eq!(diff.up_iter().filter(|(addr, _)| *addr == account2).count(), 1);
    assert_eq!(diff.down_iter().filter(|(addr, _)| *addr == account2).count(), 1);
    assert_eq!(diff.up_iter().count(), 4);
    assert_eq!(diff.down_iter().count(), 3);
}

#[test]
fn transfer_confidential_fails_with_invalid_balance() {
    let (confidential_proof, faucet_mask, _change) = generate_confidential_proof(Amount(100_000), None);
    let (mut template_test, faucet, _faucet_resx) = setup(confidential_proof);

    // Create an account
    let (account1, _owner1, _k) = template_test.create_owned_account();

    // Create proof for transfer
    let proof = generate_withdraw_proof(&faucet_mask, Amount(1001), Some(Amount(99_000)), Amount(0));

    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("account1", account1.into()),
        ("proof", ManifestValue::Value(encode(&proof.proof).unwrap())),
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
    let (confidential_proof, faucet_mask, _change) = generate_confidential_proof(Amount(100_000), None);
    let (mut template_test, faucet, faucet_resx) = setup(confidential_proof);

    // Create an account
    let (account1, owner1, _k) = template_test.create_owned_account();
    let (account2, owner2, _k) = template_test.create_owned_account();

    // Create proof for transfer

    let proof = generate_withdraw_proof(&faucet_mask, Amount(1000), Some(Amount(99_000)), Amount(0));
    // Reveal 90 tokens and 10 confidentially
    let reveal_proof = generate_withdraw_proof(&proof.output_mask, Amount(10), Some(Amount(900)), Amount(90));
    // Then reveal the rest
    let reveal_bucket_proof =
        generate_withdraw_proof(&reveal_proof.output_mask, Amount(0), Some(Amount(0)), Amount(10));

    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("resource", faucet_resx.into()),
        ("account1", account1.into()),
        ("account2", account2.into()),
        ("proof", ManifestValue::Value(encode(&proof.proof).unwrap())),
        (
            "reveal_proof",
            ManifestValue::Value(encode(&reveal_proof.proof).unwrap()),
        ),
        (
            "reveal_bucket_proof",
            ManifestValue::Value(encode(&reveal_bucket_proof.proof).unwrap()),
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

    assert_eq!(result.execution_results[12].decode::<Amount>().unwrap(), Amount(10));
    assert_eq!(result.execution_results[13].decode::<Amount>().unwrap(), Amount(90));
}

#[test]
fn attempt_to_reveal_with_unbalanced_proof() {
    let (confidential_proof, faucet_mask, _change) = generate_confidential_proof(Amount(100_000), None);
    let (mut template_test, faucet, faucet_resx) = setup(confidential_proof);

    // Create an account
    let (account1, owner1, _k) = template_test.create_owned_account();
    let (account2, _owner2, _k) = template_test.create_owned_account();

    // Create proof for transfer

    let proof = generate_withdraw_proof(&faucet_mask, Amount(1000), Some(Amount(99_000)), Amount(0));
    // Attempt to reveal more than input - change
    let reveal_proof = generate_withdraw_proof(&proof.output_mask, Amount(0), Some(Amount(900)), Amount(110));

    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("resource", faucet_resx.into()),
        ("account1", account1.into()),
        ("account2", account2.into()),
        ("proof", ManifestValue::Value(encode(&proof.proof).unwrap())),
        (
            "reveal_proof",
            ManifestValue::Value(encode(&reveal_proof.proof).unwrap()),
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
    let (confidential_proof, faucet_mask, _change) = generate_confidential_proof(Amount(100_000), None);
    let (mut template_test, faucet, faucet_resx) = setup(confidential_proof);

    // Create an account
    let (account1, owner1, _k) = template_test.create_owned_account();

    // Create proof for transfer

    let withdraw_proof1 = generate_withdraw_proof(&faucet_mask, Amount(1000), Some(Amount(99_000)), Amount(0));
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
            ManifestValue::Value(encode(&withdraw_proof1.proof).unwrap()),
        ),
        (
            "withdraw_proof2",
            ManifestValue::Value(encode(&withdraw_proof2.proof).unwrap()),
        ),
        ("join_proof", ManifestValue::Value(encode(&join_proof.proof).unwrap())),
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

    assert_eq!(result.execution_results[3].decode::<u32>().unwrap(), 1);
    assert_eq!(result.execution_results[7].decode::<u32>().unwrap(), 2);
    assert_eq!(result.execution_results[9].decode::<u32>().unwrap(), 1);
}

/// These would live in the wallet
mod utilities {
    use rand::rngs::OsRng;
    use tari_common_types::types::{BulletRangeProof, PrivateKey, PublicKey, Signature};
    use tari_crypto::{
        commitment::{ExtensionDegree, HomomorphicCommitmentFactory},
        errors::RangeProofError,
        extended_range_proof::ExtendedRangeProofService,
        keys::{PublicKey as _, SecretKey},
        ristretto::bulletproofs_plus::{RistrettoExtendedMask, RistrettoExtendedWitness},
    };
    use tari_engine_types::confidential::{challenges, get_commitment_factory, get_range_proof_service};
    use tari_template_lib::{
        crypto::{BalanceProofSignature, RistrettoPublicKeyBytes},
        models::{Amount, ConfidentialOutputProof, ConfidentialStatement, ConfidentialWithdrawProof, EncryptedValue},
    };
    use tari_utilities::ByteArray;

    pub struct ConfidentialProofStatement {
        pub amount: Amount,
        pub mask: PrivateKey,
        pub sender_public_nonce: PublicKey,
        pub minimum_value_promise: u64,
    }

    pub fn generate_confidential_proof(
        output_amount: Amount,
        change: Option<Amount>,
    ) -> (ConfidentialOutputProof, PrivateKey, Option<PrivateKey>) {
        let mask = PrivateKey::random(&mut OsRng);
        let output_statement = ConfidentialProofStatement {
            amount: output_amount,
            mask: mask.clone(),
            sender_public_nonce: Default::default(),
            minimum_value_promise: 0,
        };

        let change_mask = PrivateKey::random(&mut OsRng);
        let change_statement = change.map(|amount| ConfidentialProofStatement {
            amount,
            mask: change_mask.clone(),
            sender_public_nonce: Default::default(),
            minimum_value_promise: 0,
        });

        let proof = generate_confidential_proof_from_statements(output_statement, change_statement).unwrap();
        (proof, mask, change.map(|_| change_mask))
    }

    pub fn generate_balance_proof(
        input_mask: &PrivateKey,
        output_mask: &PrivateKey,
        change_mask: Option<&PrivateKey>,
    ) -> BalanceProofSignature {
        let secret_excess = input_mask - output_mask - change_mask.unwrap_or(&PrivateKey::default());
        let excess = PublicKey::from_secret_key(&secret_excess);
        let (nonce, public_nonce) = PublicKey::random_keypair(&mut OsRng);
        let challenge = challenges::confidential_withdraw(&excess, &public_nonce);

        let sig = Signature::sign_raw(&secret_excess, nonce, &challenge).unwrap();
        BalanceProofSignature::try_from_parts(sig.get_public_nonce().as_bytes(), sig.get_signature().as_bytes())
            .unwrap()
    }

    pub struct WithdrawProofOutput {
        pub output_mask: PrivateKey,
        pub change_mask: Option<PrivateKey>,
        pub proof: ConfidentialWithdrawProof,
    }

    pub fn generate_withdraw_proof(
        input_mask: &PrivateKey,
        output_amount: Amount,
        change_amount: Option<Amount>,
        revealed_amount: Amount,
    ) -> WithdrawProofOutput {
        let (output_proof, output_mask, change_mask) = generate_confidential_proof(output_amount, change_amount);
        let total_amount = output_amount + change_amount.unwrap_or_else(Amount::zero) + revealed_amount;
        let input_commitment = get_commitment_factory().commit_value(input_mask, total_amount.value() as u64);
        let input_commitment = copy_fixed(input_commitment.as_bytes());
        let balance_proof = generate_balance_proof(input_mask, &output_mask, change_mask.as_ref());

        let output_statement = output_proof.output_statement;
        let change_statement = output_proof.change_statement.unwrap();

        WithdrawProofOutput {
            output_mask,
            change_mask,
            proof: ConfidentialWithdrawProof {
                inputs: vec![input_commitment],
                output_proof: ConfidentialOutputProof {
                    output_statement: ConfidentialStatement {
                        commitment: output_statement.commitment,
                        sender_public_nonce: None,
                        encrypted_value: EncryptedValue::default(),
                        minimum_value_promise: output_statement.minimum_value_promise,
                    },
                    change_statement: Some(ConfidentialStatement {
                        commitment: change_statement.commitment,
                        sender_public_nonce: None,
                        encrypted_value: EncryptedValue::default(),
                        minimum_value_promise: change_statement.minimum_value_promise,
                    }),
                    range_proof: output_proof.range_proof,
                    revealed_amount,
                },
                balance_proof,
            },
        }
    }

    pub fn generate_withdraw_proof_with_inputs(
        input: &[(PrivateKey, Amount)],
        output_amount: Amount,
        change_amount: Option<Amount>,
        revealed_amount: Amount,
    ) -> WithdrawProofOutput {
        let (output_proof, output_mask, change_mask) = generate_confidential_proof(output_amount, change_amount);
        let input_commitments = input
            .iter()
            .map(|(input_mask, amount)| {
                let input_commitment = get_commitment_factory().commit_value(input_mask, amount.value() as u64);
                copy_fixed(input_commitment.as_bytes())
            })
            .collect();
        let input_private_excess = input
            .iter()
            .fold(PrivateKey::default(), |acc, (input_mask, _)| acc + input_mask);
        let balance_proof = generate_balance_proof(&input_private_excess, &output_mask, change_mask.as_ref());

        let output_statement = output_proof.output_statement;
        let change_statement = output_proof.change_statement;

        WithdrawProofOutput {
            output_mask,
            change_mask,
            proof: ConfidentialWithdrawProof {
                inputs: input_commitments,
                output_proof: ConfidentialOutputProof {
                    output_statement: ConfidentialStatement {
                        commitment: output_statement.commitment,
                        // R and encrypted value are informational and can be left out as far as the VN is concerned
                        sender_public_nonce: None,
                        encrypted_value: Default::default(),
                        minimum_value_promise: output_statement.minimum_value_promise,
                    },
                    change_statement: change_statement.map(|change| ConfidentialStatement {
                        commitment: change.commitment,
                        sender_public_nonce: None,
                        encrypted_value: Default::default(),
                        minimum_value_promise: change.minimum_value_promise,
                    }),
                    range_proof: output_proof.range_proof,
                    revealed_amount,
                },
                balance_proof,
            },
        }
    }

    fn copy_fixed<const SZ: usize>(bytes: &[u8]) -> [u8; SZ] {
        let mut array = [0u8; SZ];
        array.copy_from_slice(&bytes[..SZ]);
        array
    }

    fn generate_confidential_proof_from_statements(
        output_statement: ConfidentialProofStatement,
        change_statement: Option<ConfidentialProofStatement>,
    ) -> Result<ConfidentialOutputProof, RangeProofError> {
        let proof_change_statement = change_statement.as_ref().map(|statement| ConfidentialStatement {
            commitment: commitment_to_bytes(&statement.mask, statement.amount),
            sender_public_nonce: Some(
                RistrettoPublicKeyBytes::from_bytes(statement.sender_public_nonce.as_bytes())
                    .expect("[generate_confidential_proof] change nonce"),
            ),
            encrypted_value: Default::default(),
            minimum_value_promise: statement.minimum_value_promise,
        });

        let output_range_proof = generate_extended_bullet_proof(&output_statement, change_statement.as_ref())?;

        Ok(ConfidentialOutputProof {
            output_statement: ConfidentialStatement {
                commitment: commitment_to_bytes(&output_statement.mask, output_statement.amount),
                sender_public_nonce: Some(
                    RistrettoPublicKeyBytes::from_bytes(output_statement.sender_public_nonce.as_bytes())
                        .expect("[generate_confidential_proof] output nonce"),
                ),
                encrypted_value: Default::default(),
                minimum_value_promise: output_statement.minimum_value_promise,
            },
            change_statement: proof_change_statement,
            range_proof: output_range_proof.0,
            revealed_amount: Amount::zero(),
        })
    }

    fn generate_extended_bullet_proof(
        output_statement: &ConfidentialProofStatement,
        change_statement: Option<&ConfidentialProofStatement>,
    ) -> Result<BulletRangeProof, RangeProofError> {
        let mut extended_witnesses = vec![];

        let extended_mask =
            RistrettoExtendedMask::assign(ExtensionDegree::DefaultPedersen, vec![output_statement.mask.clone()])
                .unwrap();

        let mut agg_factor = 1;
        extended_witnesses.push(RistrettoExtendedWitness {
            mask: extended_mask,
            value: output_statement.amount.value() as u64,
            minimum_value_promise: output_statement.minimum_value_promise,
        });
        if let Some(stmt) = change_statement {
            let extended_mask =
                RistrettoExtendedMask::assign(ExtensionDegree::DefaultPedersen, vec![stmt.mask.clone()]).unwrap();
            extended_witnesses.push(RistrettoExtendedWitness {
                mask: extended_mask,
                value: stmt.amount.value() as u64,
                minimum_value_promise: stmt.minimum_value_promise,
            });
            agg_factor = 2;
        }

        let output_range_proof =
            get_range_proof_service(agg_factor).construct_extended_proof(extended_witnesses, None)?;
        Ok(BulletRangeProof(output_range_proof))
    }

    fn commitment_to_bytes(mask: &PrivateKey, amount: Amount) -> [u8; 32] {
        let commitment = get_commitment_factory().commit_value(mask, amount.value() as u64);
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(commitment.as_bytes());
        bytes
    }
}
