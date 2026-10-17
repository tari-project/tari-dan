//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_crypto::{
    keys::SecretKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey, pedersen::PedersenCommitment},
};
use tari_engine_types::{confidential::MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE, crypto::commit_amount};
use tari_ootle_wallet_crypto::{MaskAndValue, OutputWitness, confidential};
use tari_template_lib::types::{
    Amount,
    EncryptedData,
    confidential::{ConfidentialOutputStatement, ConfidentialWithdrawProof},
};

pub fn generate_confidential_output_statement(
    output_amount: u64,
    change: Option<u64>,
) -> (
    ConfidentialOutputStatement,
    RistrettoSecretKey,
    Option<RistrettoSecretKey>,
) {
    generate_confidential_proof_internal(output_amount, change, None)
}

pub fn generate_confidential_proof_with_view_key(
    output_amount: u64,
    change: Option<u64>,
    view_key: &RistrettoPublicKey,
) -> (
    ConfidentialOutputStatement,
    RistrettoSecretKey,
    Option<RistrettoSecretKey>,
) {
    generate_confidential_proof_internal(output_amount, change, Some(view_key.clone()))
}

fn generate_confidential_proof_internal(
    output_amount: u64,
    change: Option<u64>,
    view_key: Option<RistrettoPublicKey>,
) -> (
    ConfidentialOutputStatement,
    RistrettoSecretKey,
    Option<RistrettoSecretKey>,
) {
    let mut rng = rand::rng();
    let mask = RistrettoSecretKey::random(&mut rng);
    let output_statement = OutputWitness {
        amount: output_amount,
        mask: mask.clone(),
        sender_public_nonce: Default::default(),
        minimum_value_promise: MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE,
        encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
        resource_view_key: view_key.clone(),
    };

    let change_mask = RistrettoSecretKey::random(&mut rng);
    let change_statement = change.map(|amount| OutputWitness {
        amount,
        mask: change_mask.clone(),
        sender_public_nonce: Default::default(),
        minimum_value_promise: MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE,
        encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
        resource_view_key: view_key,
    });

    let proof = confidential::create_output_statement(
        Some(&output_statement),
        Amount::zero(),
        change_statement.as_ref(),
        Amount::zero(),
    )
    .unwrap();
    (proof, mask, change.map(|_| change_mask))
}

/// Generates a withdraw proof that spends a single commitment entirely into revealed funds, leaving no output
/// commitment behind.
pub fn generate_reveal_proof(input_mask: &RistrettoSecretKey, input_value: u64) -> ConfidentialWithdrawProof {
    confidential::create_withdraw_proof(
        &[MaskAndValue {
            value: input_value,
            mask: input_mask.clone(),
        }],
        Amount::zero(),
        None,
        Amount::from(input_value),
        None,
        Amount::zero(),
    )
    .unwrap()
}

pub struct ConfidentialWithdrawProofOutput {
    pub output_mask: RistrettoSecretKey,
    pub change_mask: Option<RistrettoSecretKey>,
    pub proof: ConfidentialWithdrawProof,
}

impl ConfidentialWithdrawProofOutput {
    pub fn to_commitment_for_output(&self, amount: Amount) -> Option<PedersenCommitment> {
        commit_amount(&self.output_mask, amount)
    }
}

pub fn generate_withdraw_proof<A: Into<Amount>>(
    input_mask: &RistrettoSecretKey,
    output_amount: u64,
    change_amount: Option<u64>,
    revealed_amount: A,
) -> ConfidentialWithdrawProofOutput {
    let revealed_amount = revealed_amount.into();

    let total_amount = output_amount +
        change_amount.unwrap_or(0) +
        revealed_amount.to_u64_checked().expect(
            "Revealed amount is too large to fit in u64 when generating withdraw proof. This is due to a current \
             limitation of the test tooling.",
        );

    generate_withdraw_proof_internal(
        &[(input_mask.clone(), total_amount)],
        Amount::zero(),
        output_amount,
        change_amount,
        revealed_amount,
        None,
    )
}

pub fn generate_withdraw_proof_with_inputs<A: Into<Amount>>(
    inputs: &[(RistrettoSecretKey, u64)],
    input_revealed_amount: A,
    output_amount: u64,
    change_amount: Option<u64>,
    revealed_output_amount: A,
) -> ConfidentialWithdrawProofOutput {
    generate_withdraw_proof_internal(
        inputs,
        input_revealed_amount.into(),
        output_amount,
        change_amount,
        revealed_output_amount.into(),
        None,
    )
}

pub fn generate_withdraw_proof_with_view_key<A: Into<Amount>>(
    input_mask: &RistrettoSecretKey,
    input_value: u64,
    output_amount: u64,
    change_amount: Option<u64>,
    revealed_amount: A,
    view_key: &RistrettoPublicKey,
) -> ConfidentialWithdrawProofOutput {
    generate_withdraw_proof_internal(
        &[(input_mask.clone(), input_value)],
        Amount::zero(),
        output_amount,
        change_amount,
        revealed_amount.into(),
        Some(view_key.clone()),
    )
}

fn generate_withdraw_proof_internal(
    inputs: &[(RistrettoSecretKey, u64)],
    input_revealed_amount: Amount,
    output_amount: u64,
    change_amount: Option<u64>,
    revealed_output_amount: Amount,
    view_key: Option<RistrettoPublicKey>,
) -> ConfidentialWithdrawProofOutput {
    let mut rng = rand::rng();
    // If the amount is zero, we omit the output UTXO, therefore the mask is zero
    let output_mask = if output_amount == 0 {
        RistrettoSecretKey::default()
    } else {
        RistrettoSecretKey::random(&mut rng)
    };
    let change_mask = change_amount.map(|_| RistrettoSecretKey::random(&mut rng));

    let output_proof = (output_amount > 0).then(|| OutputWitness {
        amount: output_amount,
        mask: output_mask.clone(),
        sender_public_nonce: Default::default(),
        minimum_value_promise: MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE,
        encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
        resource_view_key: view_key.clone(),
    });
    let change_proof = change_amount.map(|amount| OutputWitness {
        amount,
        mask: change_mask.clone().unwrap(),
        sender_public_nonce: Default::default(),
        minimum_value_promise: MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE,
        encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
        resource_view_key: view_key,
    });

    let proof = confidential::create_withdraw_proof(
        &inputs
            .iter()
            .map(|(mask, amount)| MaskAndValue {
                value: *amount,
                mask: mask.clone(),
            })
            .collect::<Vec<_>>(),
        input_revealed_amount,
        output_proof.as_ref(),
        revealed_output_amount,
        change_proof.as_ref(),
        Amount::zero(),
    )
    .unwrap();

    ConfidentialWithdrawProofOutput {
        output_mask,
        change_mask,
        proof,
    }
}
