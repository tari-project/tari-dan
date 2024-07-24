//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common_types::types::{BulletRangeProof, Commitment, PrivateKey, PublicKey, Signature};
use tari_crypto::{commitment::HomomorphicCommitmentFactory, tari_utilities::ByteArray};
use tari_template_lib::{
    crypto::BalanceProofSignature,
    models::{Amount, ConfidentialWithdrawProof, EncryptedData},
};

use super::{challenges, get_commitment_factory, validate_confidential_proof};
use crate::{confidential::elgamal::ElgamalVerifiableBalance, resource_container::ResourceError};

#[derive(Debug, Clone)]
pub struct ValidatedConfidentialWithdrawProof {
    /// Optional confidential output of the withdraw. This will be created as a new output commitment.
    pub output: Option<ConfidentialOutput>,
    /// Optional confidential change output of the withdraw. This will replace any inputs used.
    pub change_output: Option<ConfidentialOutput>,
    /// Range proof
    pub range_proof: BulletRangeProof,
    /// Amount of revealed value to use as an input.
    pub input_revealed_amount: Amount,
    /// Amount of revealed value to include in the revealed value of the output
    pub output_revealed_amount: Amount,
    /// Amount of revealed value to include in the revealed value of the change output
    pub change_revealed_amount: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct ConfidentialOutput {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub commitment: Commitment,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub stealth_public_nonce: PublicKey,
    #[cfg_attr(feature = "ts", ts(type = "Array<number>"))]
    pub encrypted_data: EncryptedData,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub minimum_value_promise: u64,
    pub viewable_balance: Option<ElgamalVerifiableBalance>,
}

pub(crate) fn validate_confidential_withdraw<'a, I: IntoIterator<Item = &'a Commitment>>(
    inputs: I,
    view_key: Option<&PublicKey>,
    withdraw_proof: ConfidentialWithdrawProof,
) -> Result<ValidatedConfidentialWithdrawProof, ResourceError> {
    let validated_proof = validate_confidential_proof(&withdraw_proof.output_proof, view_key)?;

    let input_revealed_amount = withdraw_proof.input_revealed_amount;
    // We expect the revealed amount to be excluded from the output commitment.
    let total_output_revealed_amount =
        withdraw_proof.output_proof.output_revealed_amount + withdraw_proof.output_proof.change_revealed_amount;

    // Balance proof not required if only revealed funds are transferred
    if withdraw_proof.is_revealed_only() {
        if input_revealed_amount.checked_sub(total_output_revealed_amount) != Some(Amount::zero()) {
            return Err(ResourceError::InvalidBalanceProof {
                details: "Incorrect balance for revealed only withdraw proof".to_string(),
            });
        }

        // This only contains revealed funds transfer, so a simple balance check is all that's needed.
        // The given zero signature _would_ be valid (public_excess == (0)), however the signature implementation
        // correctly disallows the zero key. See [ConfidentialWithdrawProof::revealed_withdraw].
        return Ok(ValidatedConfidentialWithdrawProof {
            output: None,
            change_output: validated_proof.change_output,
            range_proof: BulletRangeProof(withdraw_proof.output_proof.range_proof),
            input_revealed_amount,
            output_revealed_amount: withdraw_proof.output_proof.output_revealed_amount,
            change_revealed_amount: withdraw_proof.output_proof.change_revealed_amount,
        });
    }

    // k.G + v.H or 0.G if None
    let output_commitment = validated_proof
        .output
        .as_ref()
        .map(|o| o.commitment.as_public_key().clone())
        .unwrap_or_default();

    // 0.G + v.H
    let revealed_output_commitment =
        get_commitment_factory().commit_value(&PrivateKey::default(), total_output_revealed_amount.value() as u64);
    let output_commitment_with_revealed = output_commitment + revealed_output_commitment.as_public_key();

    let balance_proof =
        try_decode_to_signature(&withdraw_proof.balance_proof).ok_or_else(|| ResourceError::InvalidBalanceProof {
            details: "Malformed balance proof".to_string(),
        })?;

    // 0.G + v.H - users may convert revealed funds to confidential outputs so this must be part of the balance proof
    let revealed_input_commitment = get_commitment_factory().commit_value(
        &PrivateKey::default(),
        withdraw_proof.input_revealed_amount.value() as u64,
    );
    let agg_inputs = inputs
        .into_iter()
        .fold(PublicKey::default(), |sum, commit| sum + commit.as_public_key()) +
        revealed_input_commitment.as_public_key();

    let public_excess = agg_inputs -
        &output_commitment_with_revealed -
        validated_proof
            .change_output
            .as_ref()
            .map(|output| output.commitment.as_public_key())
            .unwrap_or(&PublicKey::default());

    const LOG_TARGET: &str = "tari::dan::engine::confidential::withdraw";
    log::error!(target: LOG_TARGET, "🐞public_excess: {public_excess}");
    log::error!(target: LOG_TARGET, "🐞public_nonce: {}", balance_proof.get_public_nonce());
    log::error!(target: LOG_TARGET, "🐞input_revealed_amount: {input_revealed_amount}");
    log::error!(target: LOG_TARGET, "🐞total_output_revealed_amount: {total_output_revealed_amount}");

    let challenge = challenges::confidential_withdraw64(
        &public_excess,
        balance_proof.get_public_nonce(),
        input_revealed_amount,
        total_output_revealed_amount,
    );

    if !balance_proof.verify_raw_uniform(&public_excess, &challenge) {
        return Err(ResourceError::InvalidBalanceProof {
            details: "Balance proof was invalid".to_string(),
        });
    }

    Ok(ValidatedConfidentialWithdrawProof {
        output: validated_proof.output,
        change_output: validated_proof.change_output,
        range_proof: BulletRangeProof(withdraw_proof.output_proof.range_proof),
        input_revealed_amount: withdraw_proof.input_revealed_amount,
        output_revealed_amount: withdraw_proof.output_proof.output_revealed_amount,
        change_revealed_amount: withdraw_proof.output_proof.change_revealed_amount,
    })
}

fn try_decode_to_signature(balance_proof: &BalanceProofSignature) -> Option<Signature> {
    let public_nonce = PublicKey::from_canonical_bytes(balance_proof.as_public_nonce()).ok()?;
    let signature = PrivateKey::from_canonical_bytes(balance_proof.as_signature()).ok()?;
    Some(Signature::new(public_nonce, signature))
}
