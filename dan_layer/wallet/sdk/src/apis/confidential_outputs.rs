//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::PublicKey;
use tari_crypto::dhke::DiffieHellmanSharedSecret;
use tari_dan_common_types::optional::{IsNotFoundError, Optional};

use crate::{
    apis::key_manager::{KeyManagerApi, KeyManagerApiError},
    confidential::{kdfs, ConfidentialProofError},
    models::{ConfidentialOutput, ConfidentialOutputWithMask, ConfidentialProofId},
    storage::{WalletStorageError, WalletStore, WalletStoreWriter},
};

pub struct ConfidentialOutputsApi<'a, TStore> {
    store: &'a TStore,
    key_manager_api: KeyManagerApi<'a, TStore>,
}

impl<'a, TStore: WalletStore> ConfidentialOutputsApi<'a, TStore> {
    pub fn new(store: &'a TStore, key_manager_api: KeyManagerApi<'a, TStore>) -> Self {
        Self { store, key_manager_api }
    }

    pub fn lock_outputs_by_amount(
        &self,
        account_name: &str,
        amount: u64,
        locked_by_proof_id: ConfidentialProofId,
    ) -> Result<(Vec<ConfidentialOutput>, u64), ConfidentialOutputsApiError> {
        let mut tx = self.store.create_write_tx()?;
        let mut total_output_amount = 0;
        let mut outputs = Vec::new();
        while total_output_amount < amount {
            let output = tx
                .outputs_lock_smallest_amount(account_name, locked_by_proof_id)
                .optional()?;
            match output {
                Some(output) => {
                    total_output_amount += output.value;
                    outputs.push(output);
                },
                None => {
                    tx.rollback()?;
                    return Err(ConfidentialOutputsApiError::InsufficientFunds);
                },
            }
        }
        tx.commit()?;
        Ok((outputs, total_output_amount))
    }

    pub fn add_output(&self, output: ConfidentialOutput) -> Result<(), ConfidentialOutputsApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.outputs_insert(output)?;
        tx.commit()?;
        Ok(())
    }

    pub fn add_proof(&self, account_name: String) -> Result<ConfidentialProofId, ConfidentialOutputsApiError> {
        let mut tx = self.store.create_write_tx()?;
        let proof_id = tx.proofs_insert(account_name)?;
        tx.commit()?;
        Ok(proof_id)
    }

    pub fn release_proof_outputs(&self, proof_id: ConfidentialProofId) -> Result<(), ConfidentialOutputsApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.proofs_delete(proof_id)?;
        tx.outputs_release_by_proof_id(proof_id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn finalize_outputs_for_proof(&self, proof_id: ConfidentialProofId) -> Result<(), ConfidentialOutputsApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.proofs_delete(proof_id)?;
        tx.outputs_finalize_by_proof_id(proof_id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn resolve_output_masks(
        &self,
        outputs: Vec<ConfidentialOutput>,
        account_key_branch: &str,
    ) -> Result<Vec<ConfidentialOutputWithMask>, ConfidentialOutputsApiError> {
        let mut output_masks = Vec::with_capacity(outputs.len());
        for output in outputs {
            let account_key = self
                .key_manager_api
                .derive_key(account_key_branch, output.secret_key_index)?;
            // Either derive the mask from the sender's public nonce or from the local key manager
            let mask = match output.sender_public_nonce {
                Some(nonce) => {
                    // Derive shared secret
                    let shared_secret = DiffieHellmanSharedSecret::<PublicKey>::new(&account_key.k, &nonce);
                    kdfs::output_mask_kdf(&shared_secret)
                },
                None => {
                    // Derive local secret
                    let output_mask = self
                        .key_manager_api
                        .derive_key(account_key_branch, output.secret_key_index)?;
                    output_mask.k
                },
            };

            output_masks.push(ConfidentialOutputWithMask {
                account_name: output.account_name,
                commitment: output.commitment,
                value: output.value,
                mask,
                public_asset_tag: None,
            });
        }
        Ok(output_masks)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfidentialOutputsApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Confidential proof error: {0}")]
    ConfidentialProof(#[from] ConfidentialProofError),
    #[error("Insufficient funds")]
    InsufficientFunds,
    #[error("Key manager error: {0}")]
    KeyManager(#[from] KeyManagerApiError),
}

impl IsNotFoundError for ConfidentialOutputsApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}
