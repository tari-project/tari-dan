//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::dhke::DiffieHellmanSharedSecret;
use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_engine_types::{confidential::ConfidentialOutput, substate::SubstateAddress};
use tari_key_manager::key_manager::DerivedKey;
use tari_template_lib::Hash;
use tari_utilities::ByteArray;

use crate::{
    apis::{
        accounts::{AccountsApi, AccountsApiError},
        confidential_crypto::ConfidentialCryptoApi,
        key_manager,
        key_manager::{KeyManagerApi, KeyManagerApiError},
    },
    confidential::{kdfs, ConfidentialProofError},
    models::{Account, ConfidentialOutputModel, ConfidentialOutputWithMask, ConfidentialProofId, OutputStatus},
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
        vault_address: &SubstateAddress,
        amount: u64,
        locked_by_proof_id: ConfidentialProofId,
    ) -> Result<(Vec<ConfidentialOutputModel>, u64), ConfidentialOutputsApiError> {
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
        tx.commit()?;
        Ok((outputs, total_output_amount))
    }

    pub fn add_output(&self, output: ConfidentialOutputModel) -> Result<(), ConfidentialOutputsApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.outputs_insert(output)?;
        tx.commit()?;
        Ok(())
    }

    pub fn add_proof(
        &self,
        vault_address: &SubstateAddress,
    ) -> Result<ConfidentialProofId, ConfidentialOutputsApiError> {
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
    ) -> Result<Vec<ConfidentialOutputWithMask>, ConfidentialOutputsApiError> {
        let mut output_masks = Vec::with_capacity(outputs.len());
        for output in outputs {
            let output_key = self.key_manager_api.derive_key(key_branch, output.secret_key_index)?;
            // Either derive the mask from the sender's public nonce or from the local key manager
            let mask = match output.sender_public_nonce {
                Some(nonce) => {
                    // Derive shared secret
                    let shared_secret = DiffieHellmanSharedSecret::<PublicKey>::new(&output_key.k, &nonce);
                    let shared_secret = PrivateKey::from_bytes(shared_secret.as_bytes()).unwrap();
                    kdfs::output_mask_kdf(&shared_secret)
                },
                None => {
                    // Derive local secret
                    let output_mask = self.key_manager_api.derive_key(key_branch, output.secret_key_index)?;
                    output_mask.k
                },
            };

            output_masks.push(ConfidentialOutputWithMask {
                commitment: output.commitment,
                value: output.value,
                mask,
                public_asset_tag: None,
            });
        }
        Ok(output_masks)
    }

    pub fn get_unspent_balance(&self, vault_addr: &SubstateAddress) -> Result<u64, ConfidentialOutputsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let balance = tx.outputs_get_unspent_balance(vault_addr)?;
        Ok(balance)
    }

    pub fn verify_and_update_confidential_outputs(
        &self,
        account_addr: &SubstateAddress,
        vault_addr: &SubstateAddress,
        outputs: Vec<&ConfidentialOutput>,
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
        key: &DerivedKey<PrivateKey>,
        vault_address: &SubstateAddress,
        output: &ConfidentialOutput,
    ) -> Result<ConfidentialOutputModel, ConfidentialOutputsApiError> {
        // TODO: We assumed that the wallet would know the mask and value if it produced a change output. However a
        //       wallet may be recovering funds or using a different wallet for the same accounts, so we need to
        //       provide output mask recovery and encrypted value in all cases. For now this case will error.
        let public_nonce = output
            .stealth_public_nonce
            .as_ref()
            .ok_or_else(|| ConfidentialOutputsApiError::FixMe("Missing nonce".to_string()))?;
        let encrypted_value = output
            .encrypted_value
            .as_ref()
            .ok_or_else(|| ConfidentialOutputsApiError::FixMe("Missing encrypted value".to_string()))?;

        let unblinded_result =
            self.crypto_api
                .unblind_output(&output.commitment, encrypted_value, &key.k, public_nonce);
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
            sender_public_nonce: Some(public_nonce.clone()),
            secret_key_index: account.key_index,
            public_asset_tag: None,
            status,
            locked_by_proof: None,
        })
    }

    pub fn proofs_set_transaction_hash(
        &self,
        proof_id: ConfidentialProofId,
        transaction_hash: Hash,
    ) -> Result<(), ConfidentialOutputsApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.proofs_set_transaction_hash(proof_id, transaction_hash)?;
        tx.commit()?;
        Ok(())
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
    #[error("Accounts API error: {0}")]
    Accounts(#[from] AccountsApiError),
    #[error("FIXME: {0}")]
    FixMe(String),
}

impl IsNotFoundError for ConfidentialOutputsApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}
