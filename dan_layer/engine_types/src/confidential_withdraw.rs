//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{BulletRangeProof, PrivateKey, PublicKey, Signature};
use tari_crypto::commitment::HomomorphicCommitmentFactory;
use tari_template_lib::{crypto::BalanceProofSignature, models::ConfidentialWithdrawProof};
use tari_utilities::ByteArray;

use crate::{
    confidential_validation::{validate_confidential_proof, ValidatedConfidentialWithdrawProof},
    crypto::{challenges, commitment_factory},
    resource_container::ResourceError,
};

pub fn check_confidential_withdraw<'a, I: IntoIterator<Item = &'a PublicKey>>(
    inputs: I,
    withdraw_proof: ConfidentialWithdrawProof,
) -> Result<ValidatedConfidentialWithdrawProof, ResourceError> {
    let (output_commitment, change_commitment) = validate_confidential_proof(&withdraw_proof.output_proof)?;

    // We expect the revealed amount to be excluded from the output commitment.
    let output_commitment_with_revealed = output_commitment.as_public_key() +
        commitment_factory()
            .commit_value(
                &PrivateKey::default(),
                withdraw_proof.output_proof.revealed_amount.value() as u64,
            )
            .as_public_key();

    let balance_proof =
        try_decode_to_signature(&withdraw_proof.balance_proof).ok_or_else(|| ResourceError::InvalidBalanceProof {
            details: "Malformed balance proof".to_string(),
        })?;

    let public_excess = inputs.into_iter().fold(PublicKey::default(), |sum, pk| sum + pk) -
        output_commitment_with_revealed -
        change_commitment
            .as_ref()
            .map(|c| c.as_public_key())
            .unwrap_or(&PublicKey::default());

    let challenge = challenges::confidential_withdraw(&public_excess, balance_proof.get_public_nonce());

    if !balance_proof.verify_challenge(&public_excess, &challenge) {
        return Err(ResourceError::InvalidBalanceProof {
            details: "Balance proof was invalid".to_string(),
        });
    }

    Ok(ValidatedConfidentialWithdrawProof {
        output_commitment,
        output_minimum_value_promise: withdraw_proof.output_proof.output_statement.minimum_value_promise,
        change_commitment,
        change_minimum_value_promise: withdraw_proof
            .output_proof
            .change_statement
            .map(|stmt| stmt.minimum_value_promise),
        range_proof: BulletRangeProof(withdraw_proof.output_proof.range_proof),
        revealed_amount: withdraw_proof.output_proof.revealed_amount,
    })
}

fn try_decode_to_signature(balance_proof: &BalanceProofSignature) -> Option<Signature> {
    let public_nonce = PublicKey::from_bytes(balance_proof.as_public_nonce()).ok()?;
    let signature = PrivateKey::from_bytes(balance_proof.as_signature()).ok()?;
    Some(Signature::new(public_nonce, signature))
}
