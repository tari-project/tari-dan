//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common_types::types::{BulletRangeProof, Commitment, PrivateKey, PublicKey, Signature};
use tari_crypto::commitment::HomomorphicCommitmentFactory;
use tari_template_lib::{
    crypto::BalanceProofSignature,
    models::{Amount, ConfidentialWithdrawProof, EncryptedValue},
};
use tari_utilities::ByteArray;

use super::{challenges, get_commitment_factory, validate_confidential_proof};
use crate::resource_container::ResourceError;

#[derive(Debug, Clone)]
pub struct ValidatedConfidentialWithdrawProof {
    pub output: ConfidentialOutput,
    pub change_output: Option<ConfidentialOutput>,
    pub range_proof: BulletRangeProof,
    pub output_revealed_amount: Amount,
    pub change_revealed_amount: Amount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidentialOutput {
    pub commitment: Commitment,
    pub stealth_public_nonce: Option<PublicKey>,
    pub encrypted_value: Option<EncryptedValue>,
    pub minimum_value_promise: u64,
}

pub fn validate_confidential_withdraw<'a, I: IntoIterator<Item = &'a PublicKey>>(
    inputs: I,
    withdraw_proof: ConfidentialWithdrawProof,
) -> Result<ValidatedConfidentialWithdrawProof, ResourceError> {
    let validated_proof = validate_confidential_proof(&withdraw_proof.output_proof)?;

    // We expect the revealed amount to be excluded from the output commitment.
    let revealed_amount = withdraw_proof.output_proof.output_statement.revealed_amount +
        withdraw_proof
            .output_proof
            .change_statement
            .as_ref()
            .map(|s| s.revealed_amount)
            .unwrap_or_default();
    let output_commitment_with_revealed = validated_proof.output.commitment.as_public_key() +
        get_commitment_factory()
            .commit_value(&PrivateKey::default(), revealed_amount.value() as u64)
            .as_public_key();

    let balance_proof =
        try_decode_to_signature(&withdraw_proof.balance_proof).ok_or_else(|| ResourceError::InvalidBalanceProof {
            details: "Malformed balance proof".to_string(),
        })?;

    let public_excess = inputs.into_iter().fold(PublicKey::default(), |sum, pk| sum + pk) -
        output_commitment_with_revealed -
        validated_proof
            .change_output
            .as_ref()
            .map(|output| output.commitment.as_public_key())
            .unwrap_or(&PublicKey::default());

    let challenge =
        challenges::confidential_withdraw(&public_excess, balance_proof.get_public_nonce(), revealed_amount);

    if !balance_proof.verify_challenge(&public_excess, &challenge) {
        return Err(ResourceError::InvalidBalanceProof {
            details: "Balance proof was invalid".to_string(),
        });
    }

    Ok(ValidatedConfidentialWithdrawProof {
        output: validated_proof.output,
        change_output: validated_proof.change_output,
        range_proof: BulletRangeProof(withdraw_proof.output_proof.range_proof),
        output_revealed_amount: withdraw_proof.output_proof.output_statement.revealed_amount,
        change_revealed_amount: withdraw_proof
            .output_proof
            .change_statement
            .map(|s| s.revealed_amount)
            .unwrap_or_default(),
    })
}

fn try_decode_to_signature(balance_proof: &BalanceProofSignature) -> Option<Signature> {
    let public_nonce = PublicKey::from_bytes(balance_proof.as_public_nonce()).ok()?;
    let signature = PrivateKey::from_bytes(balance_proof.as_signature()).ok()?;
    Some(Signature::new(public_nonce, signature))
}
