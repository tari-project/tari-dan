//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::encode;
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress},
    prelude::ConfidentialProof,
};
use tari_template_test_tooling::{SubstateType, TemplateTest};
use tari_transaction_manifest::ManifestValue;

use self::utilities::*;

fn setup(initial_supply: ConfidentialProof) -> (TemplateTest, ComponentAddress, SubstateAddress) {
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
    assert_eq!(total_supply, Amount(1));
}

#[test]
fn transfer_confidential_amounts_between_accounts() {
    let (confidential_proof, faucet_mask, _change) = generate_confidential_proof(Amount(100_000), None);
    let (mut template_test, faucet, faucet_resx) = setup(confidential_proof);

    // Create an account
    let (account1, owner1, _k) = template_test.create_owned_account();
    let (account2, _owner2, _k) = template_test.create_owned_account();

    // Create proof for transfer

    let proof = generate_withdraw_proof(&faucet_mask, Amount(1000), Amount(99_000));

    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("account1", account1.into()),
        ("proof", ManifestValue::Value(encode(&proof.withdraw_proof).unwrap())),
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

    let withdraw_proof = generate_withdraw_proof(&proof.output_mask, Amount(100), Amount(900));
    let split_proof = generate_withdraw_proof(&withdraw_proof.output_mask, Amount(20), Amount(80));

    let vars = [
        ("faucet_resx", faucet_resx.into()),
        ("account1", account1.into()),
        ("account2", account2.into()),
        (
            "withdraw_proof",
            ManifestValue::Value(encode(&withdraw_proof.withdraw_proof).unwrap()),
        ),
        (
            "split_proof",
            ManifestValue::Value(encode(&split_proof.withdraw_proof).unwrap()),
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

    let proof = generate_withdraw_proof(&faucet_mask, Amount(1001), Amount(99_000));

    // Transfer faucet funds into account 1
    let vars = [
        ("faucet", faucet.into()),
        ("account1", account1.into()),
        ("proof", ManifestValue::Value(encode(&proof.withdraw_proof).unwrap())),
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

/// These would live in the wallet
mod utilities {
    use rand::rngs::OsRng;
    use tari_common_types::types::{PrivateKey, PublicKey, Signature};
    use tari_crypto::keys::{PublicKey as _, SecretKey};
    use tari_engine_types::{
        crypto,
        crypto::{challenges, ConfidentialProofStatement},
    };
    use tari_template_lib::{
        crypto::BalanceProofSignature,
        models::{Amount, ConfidentialProof, ConfidentialStatement, ConfidentialWithdrawProof},
    };
    use tari_utilities::ByteArray;

    pub fn generate_confidential_proof(
        output_amount: Amount,
        change: Option<Amount>,
    ) -> (ConfidentialProof, PrivateKey, Option<PrivateKey>) {
        let mask = PrivateKey::random(&mut OsRng);

        let output_statement = ConfidentialProofStatement {
            amount: output_amount,
            mask: mask.clone(),
            minimum_value_promise: 0,
        };

        let change_mask = PrivateKey::random(&mut OsRng);
        let change_statement = change.map(|amount| ConfidentialProofStatement {
            amount,
            mask: change_mask.clone(),
            minimum_value_promise: 0,
        });

        let proof = crypto::generate_confidential_proof(output_statement, change_statement).unwrap();
        (proof, mask, change.map(|_| change_mask))
    }

    pub fn generate_balance_proof(
        input_mask: &PrivateKey,
        output_mask: &PrivateKey,
        change_mask: &PrivateKey,
    ) -> BalanceProofSignature {
        let secret_excess = input_mask - output_mask - change_mask;
        let excess = PublicKey::from_secret_key(&secret_excess);
        let (nonce, public_nonce) = PublicKey::random_keypair(&mut OsRng);
        let challenge = challenges::confidential_withdraw(&excess, &public_nonce);
        let sig = Signature::sign_raw(&secret_excess, nonce, &challenge).unwrap();
        BalanceProofSignature::try_from_parts(sig.get_public_nonce().as_bytes(), sig.get_signature().as_bytes())
            .unwrap()
    }

    pub struct WithdrawProofOutput {
        pub output_mask: PrivateKey,
        pub change_mask: PrivateKey,
        pub withdraw_proof: ConfidentialWithdrawProof,
    }

    pub fn generate_withdraw_proof(
        input_mask: &PrivateKey,
        output_amount: Amount,
        change_amount: Amount,
    ) -> WithdrawProofOutput {
        let (output_proof, output_mask, change_mask) = generate_confidential_proof(output_amount, Some(change_amount));
        let change_mask = change_mask.unwrap();
        let balance_proof = generate_balance_proof(input_mask, &output_mask, &change_mask);

        let output_statement = output_proof.output_statement;
        let change_statement = output_proof.change_statement.unwrap();

        WithdrawProofOutput {
            output_mask,
            change_mask,
            withdraw_proof: ConfidentialWithdrawProof {
                output_proof: ConfidentialProof {
                    output_statement: ConfidentialStatement {
                        commitment: output_statement.commitment,
                        minimum_value_promise: output_statement.minimum_value_promise,
                    },
                    change_statement: Some(ConfidentialStatement {
                        commitment: change_statement.commitment,
                        minimum_value_promise: change_statement.minimum_value_promise,
                    }),
                    range_proof: output_proof.range_proof,
                },
                balance_proof,
            },
        }
    }
}
