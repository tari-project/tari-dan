//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common_types::types::{BulletRangeProof, Commitment, PrivateKey, PublicKey, Signature};
use tari_crypto::{commitment::HomomorphicCommitmentFactory, tari_utilities::ByteArray};
use tari_template_lib::{
    crypto::BalanceProofSignature,
    models::{Amount, ConfidentialWithdrawProof, EncryptedData},
};
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::{challenges, get_commitment_factory, validate_confidential_proof};
use crate::resource_container::ResourceError;

#[derive(Debug, Clone)]
pub struct ValidatedConfidentialWithdrawProof {
    pub output: Option<ConfidentialOutput>,
    pub change_output: Option<ConfidentialOutput>,
    pub range_proof: BulletRangeProof,
    pub output_revealed_amount: Amount,
    pub change_revealed_amount: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ConfidentialOutput {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub commitment: Commitment,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub stealth_public_nonce: PublicKey,
    #[cfg_attr(feature = "ts", ts(type = "Array<number>"))]
    pub encrypted_data: EncryptedData,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub minimum_value_promise: u64,
}

pub fn validate_confidential_withdraw<'a, I: IntoIterator<Item = &'a Commitment>>(
    inputs: I,
    withdraw_proof: ConfidentialWithdrawProof,
) -> Result<ValidatedConfidentialWithdrawProof, ResourceError> {
    let validated_proof = validate_confidential_proof(&withdraw_proof.output_proof)?;

    // We expect the revealed amount to be excluded from the output commitment.
    let revealed_amount =
        withdraw_proof.output_proof.output_revealed_amount + withdraw_proof.output_proof.change_revealed_amount;

    // k.G + v.H or 0.G if None
    let output_commitment = validated_proof
        .output
        .as_ref()
        .map(|o| o.commitment.as_public_key().clone())
        .unwrap_or_default();

    // 0.G + v.H
    let revealed_output_commitment =
        get_commitment_factory().commit_value(&PrivateKey::default(), revealed_amount.value() as u64);
    let output_commitment_with_revealed = output_commitment + revealed_output_commitment.as_public_key();

    let balance_proof =
        try_decode_to_signature(&withdraw_proof.balance_proof).ok_or_else(|| ResourceError::InvalidBalanceProof {
            details: "Malformed balance proof".to_string(),
        })?;

    let total_inputs = inputs
        .into_iter()
        .fold(PublicKey::default(), |sum, commit| sum + commit.as_public_key());
    let public_excess = total_inputs -
        &output_commitment_with_revealed -
        validated_proof
            .change_output
            .as_ref()
            .map(|output| output.commitment.as_public_key())
            .unwrap_or(&PublicKey::default());

    let challenge =
        challenges::confidential_withdraw64(&public_excess, balance_proof.get_public_nonce(), revealed_amount);

    if !balance_proof.verify_raw_uniform(&public_excess, &challenge) {
        return Err(ResourceError::InvalidBalanceProof {
            details: "Balance proof was invalid".to_string(),
        });
    }

    Ok(ValidatedConfidentialWithdrawProof {
        output: validated_proof.output,
        change_output: validated_proof.change_output,
        range_proof: BulletRangeProof(withdraw_proof.output_proof.range_proof),
        output_revealed_amount: withdraw_proof.output_proof.output_revealed_amount,
        change_revealed_amount: withdraw_proof.output_proof.change_revealed_amount,
    })
}

fn try_decode_to_signature(balance_proof: &BalanceProofSignature) -> Option<Signature> {
    let public_nonce = PublicKey::from_canonical_bytes(balance_proof.as_public_nonce()).ok()?;
    let signature = PrivateKey::from_canonical_bytes(balance_proof.as_signature()).ok()?;
    Some(Signature::new(public_nonce, signature))
}
