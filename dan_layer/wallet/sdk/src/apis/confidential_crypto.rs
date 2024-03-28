//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::RangeInclusive;

use tari_common_types::types::{Commitment, PrivateKey, PublicKey};
use tari_dan_wallet_crypto::{
    create_confidential_proof,
    create_output_for_dest,
    create_withdraw_proof,
    encrypt_value_and_mask,
    extract_value_and_mask,
    kdfs,
    unblind_output,
    ConfidentialOutputMaskAndValue,
    ConfidentialProofError,
    ConfidentialProofStatement,
    WalletCryptoError,
};
use tari_engine_types::confidential::{ConfidentialOutput, ElgamalVerifiableBalance, ValueLookupTable};
use tari_template_lib::models::{Amount, ConfidentialOutputProof, ConfidentialWithdrawProof, EncryptedData};

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
        inputs: &[ConfidentialOutputMaskAndValue],
        input_revealed_amount: Amount,
        output_statement: &ConfidentialProofStatement,
        change_statement: Option<&ConfidentialProofStatement>,
    ) -> Result<ConfidentialWithdrawProof, ConfidentialCryptoApiError> {
        let proof = create_withdraw_proof(inputs, input_revealed_amount, output_statement, change_statement)?;
        Ok(proof)
    }

    pub fn encrypt_value_and_mask(
        &self,
        amount: u64,
        mask: &PrivateKey,
        public_nonce: &PublicKey,
        secret: &PrivateKey,
    ) -> Result<EncryptedData, ConfidentialCryptoApiError> {
        let data = encrypt_value_and_mask(amount, mask, public_nonce, secret)?;
        Ok(data)
    }

    pub fn extract_value_and_mask(
        &self,
        encryption_key: &PrivateKey,
        commitment: &Commitment,
        encrypted_data: &EncryptedData,
    ) -> Result<(u64, PrivateKey), ConfidentialCryptoApiError> {
        let value_and_mask = extract_value_and_mask(encryption_key, commitment, encrypted_data)?;
        Ok(value_and_mask)
    }

    pub fn generate_output_proof(
        &self,
        statement: &ConfidentialProofStatement,
    ) -> Result<ConfidentialOutputProof, ConfidentialCryptoApiError> {
        let proof = create_confidential_proof(statement, None)?;
        Ok(proof)
    }

    pub fn unblind_output(
        &self,
        output_commitment: &Commitment,
        output_encrypted_value: &EncryptedData,
        claim_secret: &PrivateKey,
        reciprocal_public_key: &PublicKey,
    ) -> Result<ConfidentialOutputMaskAndValue, ConfidentialCryptoApiError> {
        let unmasked_output = unblind_output(
            output_commitment,
            output_encrypted_value,
            claim_secret,
            reciprocal_public_key,
        )?;
        Ok(unmasked_output)
    }

    pub fn generate_output_for_dest(
        &self,
        dest_public_key: &PublicKey,
        amount: Amount,
    ) -> Result<ConfidentialOutput, ConfidentialCryptoApiError> {
        let output = create_output_for_dest(dest_public_key, amount)?;
        Ok(output)
    }

    pub fn try_brute_force_commitment_balances<'a, TLookup, TOutputsIter>(
        &self,
        secret_view_key: &PrivateKey,
        outputs: TOutputsIter,
        value_range: RangeInclusive<u64>,
        lookup: &mut TLookup,
    ) -> Result<Vec<Option<u64>>, TLookup::Error>
    where
        TLookup: ValueLookupTable,
        TOutputsIter: Iterator<Item = &'a ConfidentialOutput>,
    {
        ElgamalVerifiableBalance::batched_brute_force(
            secret_view_key,
            value_range,
            lookup,
            outputs.filter_map(|output| output.viewable_balance.as_ref()),
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfidentialCryptoApiError {
    #[error(transparent)]
    WalletCryptoError(#[from] WalletCryptoError),
    #[error("Confidential proof error: {0}")]
    ConfidentialProofError(#[from] ConfidentialProofError),
}
