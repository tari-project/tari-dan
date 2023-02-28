//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{Commitment, PrivateKey, PublicKey, Signature};
use tari_template_lib::{crypto::BalanceProofSignature, models::ConfidentialWithdrawProof};
use tari_utilities::ByteArray;

use crate::{
    confidential_validation::{validate_confidential_proof, ValidatedConfidentialProof},
    crypto::challenges,
    resource_container::ResourceError,
};

pub fn check_confidential_withdraw(
    input: &Commitment,
    withdraw_proof: ConfidentialWithdrawProof,
) -> Result<ValidatedConfidentialProof, ResourceError> {
    let validated_output_proof = validate_confidential_proof(withdraw_proof.output_proof)?;

    let balance_proof =
        try_decode_to_signature(&withdraw_proof.balance_proof).ok_or_else(|| ResourceError::InvalidBalanceProof {
            details: "Malformed balance proof".to_string(),
        })?;

    let public_excess = input.as_public_key() -
        validated_output_proof.output_commitment.as_public_key() -
        validated_output_proof
            .change_commitment
            .as_ref()
            .map(|c| c.as_public_key())
            .unwrap_or(&PublicKey::default());

    let challenge = challenges::confidential_withdraw(&public_excess, balance_proof.get_public_nonce());

    if !balance_proof.verify_challenge(&public_excess, &challenge) {
        return Err(ResourceError::InvalidBalanceProof {
            details: "Balance proof was invalid".to_string(),
        });
    }

    Ok(validated_output_proof)
}

fn try_decode_to_signature(balance_proof: &BalanceProofSignature) -> Option<Signature> {
    let public_nonce = PublicKey::from_bytes(balance_proof.as_public_nonce()).ok()?;
    let signature = PrivateKey::from_bytes(balance_proof.as_signature()).ok()?;
    Some(Signature::new(public_nonce, signature))
}
