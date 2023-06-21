//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chacha20poly1305::aead;
use rand::rngs::OsRng;
use tari_common_types::types::{Commitment, PrivateKey, PublicKey, Signature};
use tari_crypto::{
    commitment::HomomorphicCommitmentFactory,
    keys::{PublicKey as _, SecretKey},
};
use tari_engine_types::confidential::{challenges, ConfidentialOutput};
use tari_template_lib::{
    crypto::BalanceProofSignature,
    models::{Amount, ConfidentialOutputProof, ConfidentialWithdrawProof, EncryptedData},
};
use tari_utilities::ByteArray;

use crate::{
    byte_utils::copy_fixed,
    confidential::{
        decrypt_data_and_mask,
        encrypt_data,
        generate_confidential_proof,
        get_commitment_factory,
        kdfs,
        ConfidentialProofError,
        ConfidentialProofStatement,
    },
    models::ConfidentialOutputWithMask,
};

pub struct ConfidentialCryptoApi;

impl ConfidentialCryptoApi {
    pub(crate) fn new() -> Self {
        Self
    }

    pub fn derive_encrypted_data_key_for_receiver(
        &self,
        public_nonce: &PublicKey,
        private_key: &PrivateKey,
    ) -> PrivateKey {
        kdfs::encrypted_data_dh_kdf_aead(private_key, public_nonce)
    }

    pub fn generate_withdraw_proof(
        &self,
        inputs: &[ConfidentialOutputWithMask],
        output_statement: &ConfidentialProofStatement,
        change_statement: Option<&ConfidentialProofStatement>,
    ) -> Result<ConfidentialWithdrawProof, ConfidentialCryptoApiError> {
        let output_proof = generate_confidential_proof(output_statement, change_statement)?;
        let input_commitments = inputs
            .iter()
            .map(|input| copy_fixed(input.commitment.as_bytes()))
            .collect();

        let agg_input_mask = inputs
            .iter()
            .fold(PrivateKey::default(), |acc, output| acc + &output.mask);

        let revealed_amount = output_proof.output_statement.revealed_amount +
            output_proof
                .change_statement
                .as_ref()
                .map(|st| st.revealed_amount)
                .unwrap_or_default();
        let balance_proof = generate_balance_proof(
            &agg_input_mask,
            &output_statement.mask,
            change_statement.as_ref().map(|ch| &ch.mask),
            revealed_amount,
        );

        let output_statement = output_proof.output_statement;
        let change_statement = output_proof.change_statement;

        Ok(ConfidentialWithdrawProof {
            inputs: input_commitments,
            output_proof: ConfidentialOutputProof {
                output_statement,
                change_statement,
                range_proof: output_proof.range_proof,
            },
            balance_proof,
        })
    }

    pub fn encrypt_value_and_mask(
        &self,
        amount: u64,
        mask: &PrivateKey,
        public_nonce: &PublicKey,
        secret: &PrivateKey,
    ) -> Result<EncryptedData, ConfidentialCryptoApiError> {
        let key = kdfs::encrypted_data_dh_kdf_aead(secret, public_nonce);
        let commitment = get_commitment_factory().commit_value(mask, amount);
        let encrypted_data = encrypt_data(&key, &commitment, amount, mask)?;
        Ok(encrypted_data)
    }

    pub fn extract_value_and_mask(
        &self,
        encryption_key: &PrivateKey,
        commitment: &Commitment,
        encrypted_data: &EncryptedData,
    ) -> Result<(u64, PrivateKey), ConfidentialCryptoApiError> {
        let (value, mask) = decrypt_data_and_mask(encryption_key, commitment, encrypted_data)
            .map_err(|_| ConfidentialCryptoApiError::FailedDecryptValue)?;
        Ok((value, mask))
    }

    pub fn generate_output_proof(
        &self,
        statement: &ConfidentialProofStatement,
    ) -> Result<ConfidentialOutputProof, ConfidentialCryptoApiError> {
        let proof = generate_confidential_proof(statement, None)?;
        Ok(proof)
    }

    pub fn unblind_output(
        &self,
        output_commitment: &Commitment,
        output_encrypted_value: &EncryptedData,
        claim_secret: &PrivateKey,
        reciprocal_public_key: &PublicKey,
    ) -> Result<ConfidentialOutputWithMask, ConfidentialCryptoApiError> {
        let encryption_key = self.derive_encrypted_data_key_for_receiver(reciprocal_public_key, claim_secret);

        let (value, mask) = self.extract_value_and_mask(&encryption_key, output_commitment, output_encrypted_value)?;
        let commitment = get_commitment_factory().commit_value(&mask, value);
        if *output_commitment == commitment {
            Ok(ConfidentialOutputWithMask {
                commitment,
                value,
                mask,
                public_asset_tag: None,
            })
        } else {
            Err(ConfidentialCryptoApiError::UnableToOpenCommitment)
        }
    }

    pub fn generate_output_for_dest(
        &self,
        dest_public_key: &PublicKey,
        amount: Amount,
    ) -> Result<ConfidentialOutput, ConfidentialCryptoApiError> {
        let mask = PrivateKey::random(&mut OsRng);
        let stealth_public_nonce = PublicKey::from_secret_key(&mask);
        let amount = amount
            .as_u64_checked()
            .ok_or_else(|| ConfidentialCryptoApiError::InvalidArgument {
                name: "amount",
                details: "[generate_output_for_dest] amount is negative".to_string(),
            })?;
        let commitment = self.create_commitment(&mask, amount);
        let encrypt_key = self.derive_encrypted_data_key_for_receiver(dest_public_key, &mask);
        let encrypted_data = encrypt_data(&encrypt_key, &commitment, amount, &mask)?;

        Ok(ConfidentialOutput {
            commitment,
            stealth_public_nonce,
            encrypted_data,
            minimum_value_promise: 0,
        })
    }

    fn create_commitment(&self, mask: &PrivateKey, value: u64) -> Commitment {
        get_commitment_factory().commit_value(mask, value)
    }
}

fn generate_balance_proof(
    input_mask: &PrivateKey,
    output_mask: &PrivateKey,
    change_mask: Option<&PrivateKey>,
    reveal_amount: Amount,
) -> BalanceProofSignature {
    let secret_excess = input_mask - output_mask - change_mask.unwrap_or(&PrivateKey::default());
    let excess = PublicKey::from_secret_key(&secret_excess);
    let (nonce, public_nonce) = PublicKey::random_keypair(&mut OsRng);
    let challenge = challenges::confidential_withdraw(&excess, &public_nonce, reveal_amount);

    let sig = Signature::sign_raw(&secret_excess, nonce, &challenge).unwrap();
    BalanceProofSignature::try_from_parts(sig.get_public_nonce().as_bytes(), sig.get_signature().as_bytes()).unwrap()
}

#[derive(Debug, thiserror::Error)]
pub enum ConfidentialCryptoApiError {
    #[error("Confidential proof error: {0}")]
    ConfidentialProof(#[from] ConfidentialProofError),
    #[error("Failed to decrypt value")]
    FailedDecryptValue,
    #[error("Unable to open the commitment")]
    UnableToOpenCommitment,
    #[error("Invalid argument {name}: {details}")]
    InvalidArgument { name: &'static str, details: String },
    #[error("AEAD error: {0}")]
    AeadError(aead::Error),
}

impl From<aead::Error> for ConfidentialCryptoApiError {
    fn from(err: aead::Error) -> Self {
        ConfidentialCryptoApiError::AeadError(err)
    }
}
