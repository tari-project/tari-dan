//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use chacha20poly1305::aead;
use rand::rngs::OsRng;
use tari_crypto::{
    commitment::HomomorphicCommitmentFactory,
    keys::{PublicKey as _, SecretKey},
    ristretto::{pedersen::PedersenCommitment, RistrettoPublicKey, RistrettoSchnorr, RistrettoSecretKey},
    tari_utilities::ByteArray,
};
use tari_engine_types::confidential::{challenges, get_commitment_factory, ConfidentialOutput};
use tari_template_lib::{
    crypto::{BalanceProofSignature, PedersonCommitmentBytes},
    models::{Amount, ConfidentialOutputStatement, ConfidentialWithdrawProof, EncryptedData},
};

use crate::{
    confidential_output::ConfidentialOutputMaskAndValue,
    kdfs,
    proof::{create_confidential_output_statement, decrypt_data_and_mask, encrypt_data},
    ConfidentialProofError,
    ConfidentialProofStatement,
};

pub fn create_withdraw_proof(
    inputs: &[ConfidentialOutputMaskAndValue],
    input_revealed_amount: Amount,
    output_statement: Option<&ConfidentialProofStatement>,
    output_revealed_amount: Amount,
    change_statement: Option<&ConfidentialProofStatement>,
    change_revealed_amount: Amount,
) -> Result<ConfidentialWithdrawProof, WalletCryptoError> {
    let output_proof = create_confidential_output_statement(
        output_statement,
        output_revealed_amount,
        change_statement,
        change_revealed_amount,
    )?;
    let (input_commitments, agg_input_mask) = inputs.iter().fold(
        (Vec::with_capacity(inputs.len()), RistrettoSecretKey::default()),
        |(mut commitments, agg_input), input| {
            let commitment = get_commitment_factory().commit_value(&input.mask, input.value);
            commitments.push(
                PedersonCommitmentBytes::from_bytes(commitment.as_bytes()).expect("PedersonCommitment not 32 bytes"),
            );
            (commitments, agg_input + &input.mask)
        },
    );

    let output_revealed_amount = output_proof.output_revealed_amount + output_proof.change_revealed_amount;
    let balance_proof = generate_balance_proof(
        &agg_input_mask,
        input_revealed_amount,
        output_statement.as_ref().map(|o| &o.mask),
        change_statement.as_ref().map(|ch| &ch.mask),
        output_revealed_amount,
    );

    let output_statement = output_proof.output_statement;
    let change_statement = output_proof.change_statement;

    Ok(ConfidentialWithdrawProof {
        inputs: input_commitments,
        input_revealed_amount,
        output_proof: ConfidentialOutputStatement {
            output_statement,
            change_statement,
            range_proof: output_proof.range_proof,
            output_revealed_amount: output_proof.output_revealed_amount,
            change_revealed_amount: output_proof.change_revealed_amount,
        },
        balance_proof,
    })
}

pub fn encrypt_value_and_mask(
    amount: u64,
    mask: &RistrettoSecretKey,
    public_nonce: &RistrettoPublicKey,
    secret: &RistrettoSecretKey,
) -> Result<EncryptedData, WalletCryptoError> {
    let key = kdfs::encrypted_data_dh_kdf_aead(secret, public_nonce);
    let commitment = get_commitment_factory().commit_value(mask, amount);
    let encrypted_data = encrypt_data(&key, &commitment, amount, mask)?;
    Ok(encrypted_data)
}

pub fn extract_value_and_mask(
    encryption_key: &RistrettoSecretKey,
    commitment: &PedersenCommitment,
    encrypted_data: &EncryptedData,
) -> Result<(u64, RistrettoSecretKey), WalletCryptoError> {
    let (value, mask) = decrypt_data_and_mask(encryption_key, commitment, encrypted_data)
        .map_err(|e| WalletCryptoError::FailedDecryptData { details: e.to_string() })?;
    Ok((value, mask))
}

pub fn unblind_output(
    output_commitment: &PedersenCommitment,
    output_encrypted_value: &EncryptedData,
    claim_secret: &RistrettoSecretKey,
    reciprocal_public_key: &RistrettoPublicKey,
) -> Result<ConfidentialOutputMaskAndValue, WalletCryptoError> {
    let encryption_key = kdfs::encrypted_data_dh_kdf_aead(claim_secret, reciprocal_public_key);

    let (value, mask) = extract_value_and_mask(&encryption_key, output_commitment, output_encrypted_value)?;
    let commitment = get_commitment_factory().commit_value(&mask, value);
    if *output_commitment == commitment {
        Ok(ConfidentialOutputMaskAndValue { value, mask })
    } else {
        Err(WalletCryptoError::UnableToOpenCommitment)
    }
}

pub fn create_output_for_dest(
    dest_public_key: &RistrettoPublicKey,
    amount: Amount,
) -> Result<ConfidentialOutput, WalletCryptoError> {
    let mask = RistrettoSecretKey::random(&mut OsRng);
    // FIXME: This allows anyone to subtract the public mask from the commitment and brute force the value
    // This is only used for create free test coins
    let stealth_public_nonce = RistrettoPublicKey::from_secret_key(&mask);
    let amount = amount
        .as_u64_checked()
        .ok_or_else(|| WalletCryptoError::InvalidArgument {
            name: "amount",
            details: "[generate_output_for_dest] amount is negative".to_string(),
        })?;
    let commitment = create_commitment(&mask, amount);
    let encrypt_key = kdfs::encrypted_data_dh_kdf_aead(&mask, dest_public_key);
    let encrypted_data = encrypt_data(&encrypt_key, &commitment, amount, &mask)?;

    Ok(ConfidentialOutput {
        commitment,
        stealth_public_nonce,
        encrypted_data,
        minimum_value_promise: 0,
        viewable_balance: None,
    })
}

fn create_commitment(mask: &RistrettoSecretKey, value: u64) -> PedersenCommitment {
    get_commitment_factory().commit_value(mask, value)
}

fn generate_balance_proof(
    input_mask: &RistrettoSecretKey,
    input_revealed_amount: Amount,
    output_mask: Option<&RistrettoSecretKey>,
    change_mask: Option<&RistrettoSecretKey>,
    output_reveal_amount: Amount,
) -> BalanceProofSignature {
    let secret_excess = input_mask -
        output_mask.unwrap_or(&RistrettoSecretKey::default()) -
        change_mask.unwrap_or(&RistrettoSecretKey::default());
    if secret_excess == RistrettoSecretKey::default() {
        // This is a revealed only proof
        return BalanceProofSignature::zero();
    }
    let excess = RistrettoPublicKey::from_secret_key(&secret_excess);
    let (nonce, public_nonce) = RistrettoPublicKey::random_keypair(&mut OsRng);
    let message =
        challenges::confidential_withdraw64(&excess, &public_nonce, input_revealed_amount, output_reveal_amount);

    let sig = RistrettoSchnorr::sign_raw_uniform(&secret_excess, nonce, &message).unwrap();
    BalanceProofSignature::try_from_parts(sig.get_public_nonce().as_bytes(), sig.get_signature().as_bytes()).unwrap()
}

#[derive(Debug, thiserror::Error)]
pub enum WalletCryptoError {
    #[error("Confidential proof error: {0}")]
    ConfidentialProof(#[from] ConfidentialProofError),
    #[error("Failed to decrypt data: {details}")]
    FailedDecryptData { details: String },
    #[error("Unable to open the commitment")]
    UnableToOpenCommitment,
    #[error("Invalid argument {name}: {details}")]
    InvalidArgument { name: &'static str, details: String },
    #[error("AEAD error: {0}")]
    AeadError(aead::Error),
}

impl From<aead::Error> for WalletCryptoError {
    fn from(err: aead::Error) -> Self {
        WalletCryptoError::AeadError(err)
    }
}
