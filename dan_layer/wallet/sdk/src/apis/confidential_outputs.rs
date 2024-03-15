//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common_types::types::PublicKey;
use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_dan_wallet_crypto::{kdfs, ConfidentialOutputMaskAndValue};
use tari_engine_types::{confidential::ConfidentialOutput, substate::SubstateId};
use tari_key_manager::key_manager::DerivedKey;
use tari_template_lib::models::Amount;
use tari_transaction::TransactionId;

use crate::{
    apis::{
        accounts::{AccountsApi, AccountsApiError},
        confidential_crypto::{ConfidentialCryptoApi, ConfidentialCryptoApiError},
        key_manager,
        key_manager::{KeyManagerApi, KeyManagerApiError},
    },
    models::{Account, ConfidentialOutputModel, ConfidentialProofId, OutputStatus},
    storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter},
};

const LOG_TARGET: &str = "tari::dan::wallet_sdk::apis::confidential_outputs";

pub struct ConfidentialOutputsApi<'a, TStore> {
    store: &'a TStore,
    key_manager_api: KeyManagerApi<'a, TStore>,
    accounts_api: AccountsApi<'a, TStore>,
    crypto_api: ConfidentialCryptoApi,
}

impl<'a, TStore: WalletStore> ConfidentialOutputsApi<'a, TStore> {
    pub fn new(
        store: &'a TStore,
        key_manager_api: KeyManagerApi<'a, TStore>,
        accounts_api: AccountsApi<'a, TStore>,
        crypto_api: ConfidentialCryptoApi,
    ) -> Self {
        Self {
            store,
            key_manager_api,
            accounts_api,
            crypto_api,
        }
    }

    pub fn lock_outputs_by_amount(
        &self,
        vault_address: &SubstateId,
        amount: Amount,
        locked_by_proof_id: ConfidentialProofId,
        dry_run: bool,
    ) -> Result<(Vec<ConfidentialOutputModel>, u64), ConfidentialOutputsApiError> {
        if amount.is_negative() {
            return Err(ConfidentialOutputsApiError::InvalidParameter {
                param: "amount",
                reason: "Amount cannot be negative".to_string(),
            });
        }
        let amount = amount.as_u64_checked().unwrap();
        let mut tx = self.store.create_write_tx()?;
        let mut total_output_amount = 0;
        let mut outputs = Vec::new();
        while total_output_amount < amount {
            let output = tx
                .outputs_lock_smallest_amount(vault_address, locked_by_proof_id)
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
        if dry_run {
            tx.rollback()?;
        } else {
            tx.commit()?;
        }
        Ok((outputs, total_output_amount))
    }

    pub fn add_output(&self, output: ConfidentialOutputModel) -> Result<(), ConfidentialOutputsApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.outputs_insert(output)?;
        tx.commit()?;
        Ok(())
    }

    pub fn add_proof(&self, vault_address: &SubstateId) -> Result<ConfidentialProofId, ConfidentialOutputsApiError> {
        let mut tx = self.store.create_write_tx()?;
        let proof_id = tx.proofs_insert(vault_address)?;
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
        outputs: Vec<ConfidentialOutputModel>,
        key_branch: &str,
    ) -> Result<Vec<ConfidentialOutputMaskAndValue>, ConfidentialOutputsApiError> {
        let mut outputs_with_masks = Vec::with_capacity(outputs.len());
        for output in outputs {
            let output_key = self
                .key_manager_api
                .derive_key(key_branch, output.encryption_secret_key_index)?;
            // Either derive the mask from the sender's public nonce or from the local key manager
            let shared_decrypt_key = match output.sender_public_nonce {
                Some(nonce) => {
                    // Derive shared secret
                    kdfs::encrypted_data_dh_kdf_aead(&output_key.key, &nonce)
                },
                None => {
                    // Derive local secret
                    let output_key = self
                        .key_manager_api
                        .derive_key(key_branch, output.encryption_secret_key_index)?;
                    output_key.key
                },
            };

            let (_, mask) = self.crypto_api.extract_value_and_mask(
                &shared_decrypt_key,
                &output.commitment,
                &output.encrypted_data,
            )?;

            outputs_with_masks.push(ConfidentialOutputMaskAndValue {
                value: output.value,
                mask,
            });
        }
        Ok(outputs_with_masks)
    }

    pub fn get_unspent_balance(&self, vault_addr: &SubstateId) -> Result<u64, ConfidentialOutputsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let balance = tx.outputs_get_unspent_balance(vault_addr)?;
        Ok(balance)
    }

    pub fn verify_and_update_confidential_outputs<'i, I: IntoIterator<Item = &'i ConfidentialOutput>>(
        &self,
        account_addr: &SubstateId,
        vault_addr: &SubstateId,
        outputs: I,
    ) -> Result<(), ConfidentialOutputsApiError> {
        let account = self.accounts_api.get_account_by_address(account_addr)?;
        // We do not support changing of account key at this time
        let key = self
            .key_manager_api
            .derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
        let mut tx = self.store.create_write_tx()?;
        for output in outputs {
            match tx.outputs_get_by_commitment(&output.commitment).optional()? {
                Some(_) => {
                    info!(
                        target: LOG_TARGET,
                        "Output already exists in the wallet. Skipping. (commitment: {})",
                        output.commitment.as_public_key()
                    );
                    // Output exists. We should never have the case this is marked as spent. Should we check that?
                },
                None => {
                    // Output does not exist. Add it to the store
                    match self.validate_output(&account, &key, vault_addr, output) {
                        Ok(output) => {
                            tx.outputs_insert(output)?;
                        },
                        Err(e) => {
                            warn!(
                                target: LOG_TARGET,
                                "Output validation failed. Skipping. (commitment: {}, error: {})",
                                output.commitment.as_public_key(),
                                e
                            );
                        },
                    }
                },
            }
        }
        tx.commit()?;

        Ok(())
    }

    fn validate_output(
        &self,
        account: &Account,
        key: &DerivedKey<PublicKey>,
        vault_address: &SubstateId,
        output: &ConfidentialOutput,
    ) -> Result<ConfidentialOutputModel, ConfidentialOutputsApiError> {
        let unblinded_result = self.crypto_api.unblind_output(
            &output.commitment,
            &output.encrypted_data,
            &key.key,
            &output.stealth_public_nonce,
        );
        let (value, status) = match unblinded_result {
            Ok(output) => (output.value, OutputStatus::Unspent),
            Err(e) => {
                warn!(
                    target: LOG_TARGET,
                    "Failed to unblind output. (commitment: {}, error: {})",
                    output.commitment.as_public_key(),
                    e
                );
                (0, OutputStatus::Invalid)
            },
        };

        Ok(ConfidentialOutputModel {
            account_address: account.address.clone(),
            vault_address: vault_address.clone(),
            commitment: output.commitment.clone(),
            value,
            sender_public_nonce: Some(output.stealth_public_nonce.clone()),
            encryption_secret_key_index: account.key_index,
            encrypted_data: output.encrypted_data.clone(),
            public_asset_tag: None,
            status,
            locked_by_proof: None,
        })
    }

    pub fn proofs_set_transaction_hash(
        &self,
        proof_id: ConfidentialProofId,
        transaction_id: TransactionId,
    ) -> Result<(), ConfidentialOutputsApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.proofs_set_transaction_id(proof_id, transaction_id)?;
        tx.commit()?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfidentialOutputsApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Confidential crypto error: {0}")]
    ConfidentialCrypto(#[from] ConfidentialCryptoApiError),
    #[error("Insufficient funds")]
    InsufficientFunds,
    #[error("Key manager error: {0}")]
    KeyManager(#[from] KeyManagerApiError),
    #[error("Accounts API error: {0}")]
    Accounts(#[from] AccountsApiError),
    #[error("Invalid parameter `{param}`: {reason}")]
    InvalidParameter { param: &'static str, reason: String },
}

impl IsNotFoundError for ConfidentialOutputsApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}
