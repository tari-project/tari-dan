//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chacha20poly1305::aead;
use rand::rngs::OsRng;
use tari_common_types::types::{Commitment, PrivateKey, PublicKey, Signature};
use tari_crypto::{
    commitment::HomomorphicCommitmentFactory,
    dhke::DiffieHellmanSharedSecret,
    keys::PublicKey as _,
    ristretto::RistrettoPublicKey,
};
use tari_engine_types::confidential::{challenges, ConfidentialOutput};
use tari_template_lib::{
    crypto::BalanceProofSignature,
    models::{Amount, ConfidentialOutputProof, ConfidentialWithdrawProof, EncryptedValue},
};
use tari_utilities::ByteArray;

use crate::{
    byte_utils::copy_fixed,
    confidential::{
        decrypt_value,
        encrypt_value,
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

    pub fn derive_output_mask_for_destination(&self, destination_public_key: &PublicKey) -> (PrivateKey, PublicKey) {
        let (nonce, public_nonce) = PublicKey::random_keypair(&mut OsRng);
        let shared_secret = DiffieHellmanSharedSecret::<PublicKey>::new(&nonce, destination_public_key);
        let shared_secret = PrivateKey::from_bytes(shared_secret.as_bytes()).unwrap();
        let shared_secret = kdfs::output_mask_kdf(&shared_secret);
        (shared_secret, public_nonce)
    }

    pub fn derive_output_mask_for_receiver(&self, public_nonce: &PublicKey, secret_key: &PrivateKey) -> PrivateKey {
        let shared_secret = DiffieHellmanSharedSecret::<PublicKey>::new(secret_key, public_nonce);
        let shared_secret = PrivateKey::from_bytes(shared_secret.as_bytes()).unwrap();
        kdfs::output_mask_kdf(&shared_secret)
    }

    pub fn derive_value_encryption_key_for_receiver(
        &self,
        private_key: &PrivateKey,
        commitment: &Commitment,
    ) -> PrivateKey {
        kdfs::encrypted_value_kdf_aead(private_key, commitment)
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
            .map(|input| {
                let input_commitment = self.create_commitment(&input.mask, input.value);
                copy_fixed(input_commitment.as_bytes())
            })
            .collect();
        let input_private_excess = inputs
            .iter()
            .fold(PrivateKey::default(), |acc, output| acc + &output.mask);

        let revealed_amount = output_proof.output_statement.revealed_amount +
            output_proof
                .change_statement
                .as_ref()
                .map(|st| st.revealed_amount)
                .unwrap_or_default();
        let balance_proof = generate_balance_proof(
            &input_private_excess,
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

    pub fn extract_value(
        &self,
        encryption_key: &PrivateKey,
        commitment: &Commitment,
        encrypted_value: &EncryptedValue,
    ) -> Result<u64, ConfidentialCryptoApiError> {
        let value = decrypt_value(encryption_key, commitment, encrypted_value)
            .map_err(|_| ConfidentialCryptoApiError::FailedDecryptValue)?;
        Ok(value)
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
        output_encrypted_value: &EncryptedValue,
        claim_secret: &PrivateKey,
        reciprocal_public_key: &PublicKey,
    ) -> Result<ConfidentialOutputWithMask, ConfidentialCryptoApiError> {
        let mask = self.derive_output_mask_for_receiver(reciprocal_public_key, claim_secret);
        let encryption_key = self.derive_value_encryption_key_for_receiver(&mask, output_commitment);

        let value = self.extract_value(&encryption_key, output_commitment, output_encrypted_value)?;
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
        dest_public_key: &RistrettoPublicKey,
        amount: Amount,
    ) -> Result<ConfidentialOutput, ConfidentialCryptoApiError> {
        let (output_mask, pub_nonce) = self.derive_output_mask_for_destination(dest_public_key);
        let amount = amount
            .value()
            .try_into()
            .map_err(|_| ConfidentialCryptoApiError::InvalidArgument {
                name: "amount",
                details: "[generate_output_for_dest] amount is negative".to_string(),
            })?;
        let commitment = self.create_commitment(&output_mask, amount);
        let encrypt_key = self.derive_value_encryption_key_for_receiver(&output_mask, &commitment);
        let encrypted_value =
            encrypt_value(&encrypt_key, &commitment, amount).map_err(ConfidentialCryptoApiError::AeadError)?;

        Ok(ConfidentialOutput {
            commitment,
            stealth_public_nonce: pub_nonce,
            encrypted_value,
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
