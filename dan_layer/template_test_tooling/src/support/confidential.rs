//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::{commitment::HomomorphicCommitmentFactory, keys::SecretKey, tari_utilities::ByteArray};
use tari_dan_wallet_crypto::{ConfidentialOutputMaskAndValue, ConfidentialProofStatement};
use tari_engine_types::confidential::get_commitment_factory;
use tari_template_lib::{
    crypto::PedersonCommitmentBytes,
    models::{Amount, ConfidentialOutputStatement, ConfidentialWithdrawProof},
};

pub fn generate_confidential_proof(
    output_amount: Amount,
    change: Option<Amount>,
) -> (ConfidentialOutputStatement, PrivateKey, Option<PrivateKey>) {
    generate_confidential_proof_internal(output_amount, change, None)
}

pub fn generate_confidential_proof_with_view_key(
    output_amount: Amount,
    change: Option<Amount>,
    view_key: &PublicKey,
) -> (ConfidentialOutputStatement, PrivateKey, Option<PrivateKey>) {
    generate_confidential_proof_internal(output_amount, change, Some(view_key.clone()))
}

fn generate_confidential_proof_internal(
    output_amount: Amount,
    change: Option<Amount>,
    view_key: Option<PublicKey>,
) -> (ConfidentialOutputStatement, PrivateKey, Option<PrivateKey>) {
    let mask = PrivateKey::random(&mut OsRng);
    let output_statement = ConfidentialProofStatement {
        amount: output_amount,
        mask: mask.clone(),
        sender_public_nonce: Default::default(),
        minimum_value_promise: 0,
        encrypted_data: Default::default(),
        reveal_amount: Default::default(),
        resource_view_key: view_key.clone(),
    };

    let change_mask = PrivateKey::random(&mut OsRng);
    let change_statement = change.map(|amount| ConfidentialProofStatement {
        amount,
        mask: change_mask.clone(),
        sender_public_nonce: Default::default(),
        minimum_value_promise: 0,
        encrypted_data: Default::default(),
        reveal_amount: Default::default(),
        resource_view_key: view_key,
    });

    let proof =
        tari_dan_wallet_crypto::create_confidential_output_statement(&output_statement, change_statement.as_ref())
            .unwrap();
    (proof, mask, change.map(|_| change_mask))
}

pub struct WithdrawProofOutput {
    pub output_mask: PrivateKey,
    pub change_mask: Option<PrivateKey>,
    pub proof: ConfidentialWithdrawProof,
}

impl WithdrawProofOutput {
    pub fn to_commitment_bytes_for_output(&self, amount: Amount) -> PedersonCommitmentBytes {
        let commitment = get_commitment_factory().commit_value(&self.output_mask, amount.value() as u64);
        PedersonCommitmentBytes::from_bytes(commitment.as_bytes()).unwrap()
    }
}

pub fn generate_withdraw_proof(
    input_mask: &PrivateKey,
    output_amount: Amount,
    change_amount: Option<Amount>,
    revealed_amount: Amount,
) -> WithdrawProofOutput {
    let total_amount = output_amount + change_amount.unwrap_or_else(Amount::zero) + revealed_amount;

    generate_withdraw_proof_internal(
        &[(input_mask.clone(), total_amount)],
        Amount::zero(),
        output_amount,
        change_amount,
        revealed_amount,
        None,
    )
}

pub fn generate_withdraw_proof_with_inputs(
    inputs: &[(PrivateKey, Amount)],
    input_revealed_amount: Amount,
    output_amount: Amount,
    change_amount: Option<Amount>,
    revealed_output_amount: Amount,
) -> WithdrawProofOutput {
    generate_withdraw_proof_internal(
        inputs,
        input_revealed_amount,
        output_amount,
        change_amount,
        revealed_output_amount,
        None,
    )
}

pub fn generate_withdraw_proof_with_view_key(
    input_mask: &PrivateKey,
    input_value: Amount,
    output_amount: Amount,
    change_amount: Option<Amount>,
    revealed_amount: Amount,
    view_key: &PublicKey,
) -> WithdrawProofOutput {
    generate_withdraw_proof_internal(
        &[(input_mask.clone(), input_value)],
        Amount::zero(),
        output_amount,
        change_amount,
        revealed_amount,
        Some(view_key.clone()),
    )
}

fn generate_withdraw_proof_internal(
    inputs: &[(PrivateKey, Amount)],
    input_revealed_amount: Amount,
    output_amount: Amount,
    change_amount: Option<Amount>,
    revealed_output_amount: Amount,
    view_key: Option<PublicKey>,
) -> WithdrawProofOutput {
    // If the amount is zero, we omit the output UTXO, therefore the mask is zero
    let output_mask = if output_amount.is_zero() {
        Default::default()
    } else {
        PrivateKey::random(&mut OsRng)
    };
    let change_mask = change_amount.map(|_| PrivateKey::random(&mut OsRng));

    let output_proof = ConfidentialProofStatement {
        amount: output_amount,
        mask: output_mask.clone(),
        sender_public_nonce: Default::default(),
        minimum_value_promise: 0,
        encrypted_data: Default::default(),
        reveal_amount: revealed_output_amount,
        resource_view_key: view_key.clone(),
    };
    let change_proof = change_amount.map(|amount| ConfidentialProofStatement {
        amount,
        mask: change_mask.clone().unwrap(),
        sender_public_nonce: Default::default(),
        minimum_value_promise: 0,
        encrypted_data: Default::default(),
        reveal_amount: Default::default(),
        resource_view_key: view_key,
    });

    let proof = tari_dan_wallet_crypto::create_withdraw_proof(
        &inputs
            .iter()
            .map(|(mask, amount)| ConfidentialOutputMaskAndValue {
                value: amount.as_u64_checked().unwrap(),
                mask: mask.clone(),
            })
            .collect::<Vec<_>>(),
        input_revealed_amount,
        &output_proof,
        change_proof.as_ref(),
    )
    .unwrap();

    WithdrawProofOutput {
        output_mask,
        change_mask,
        proof,
    }
}
