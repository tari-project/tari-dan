//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::{Deref, DerefMut};

use tari_common_types::types::{Commitment, FixedHash, PublicKey};
use tari_dan_common_types::{optional::IsNotFoundError, QuorumCertificate};
use tari_engine_types::{
    commit_result::{FinalizeResult, RejectReason},
    substate::SubstateAddress,
    TemplateAddress,
};
use tari_template_lib::{models::Amount, prelude::ResourceAddress, Hash};
use tari_transaction::Transaction;

use crate::models::{
    Account,
    ConfidentialOutputModel,
    ConfidentialProofId,
    Config,
    OutputStatus,
    SubstateModel,
    TransactionStatus,
    VaultModel,
    VersionedSubstateAddress,
    WalletTransaction,
};

pub trait WalletStore {
    type ReadTransaction<'a>: WalletStoreReader
    where Self: 'a;
    type WriteTransaction<'a>: WalletStoreWriter + Deref<Target = Self::ReadTransaction<'a>> + DerefMut
    where Self: 'a;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, WalletStorageError>;
    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, WalletStorageError>;

    fn with_write_tx<F: FnOnce(&mut Self::WriteTransaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where E: From<WalletStorageError> {
        let mut tx = self.create_write_tx()?;
        match f(&mut tx) {
            Ok(r) => {
                tx.commit()?;
                Ok(r)
            },
            Err(e) => {
                if let Err(err) = tx.rollback() {
                    log::error!("Failed to rollback transaction: {}", err);
                }
                Err(e)
            },
        }
    }

    fn with_read_tx<F: FnOnce(&mut Self::ReadTransaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where E: From<WalletStorageError> {
        let mut tx = self.create_read_tx()?;
        let ret = f(&mut tx)?;
        Ok(ret)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WalletStorageError {
    #[error("General database failure for operation {operation}: {details}")]
    GeneralFailure { operation: &'static str, details: String },
    #[error("Failed to decode for operation {operation} on {item}: {details}")]
    DecodingError {
        operation: &'static str,
        item: &'static str,
        details: String,
    },
    #[error("Failed to encode for operation {operation} on {item}: {details}")]
    EncodingError {
        operation: &'static str,
        item: &'static str,
        details: String,
    },
    #[error("{entity} not found with key {key}")]
    NotFound {
        operation: &'static str,
        entity: String,
        key: String,
    },
}

impl IsNotFoundError for WalletStorageError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::NotFound { .. })
    }
}

impl WalletStorageError {
    pub fn general<E: std::fmt::Display>(operation: &'static str, e: E) -> Self {
        Self::GeneralFailure {
            operation,
            details: e.to_string(),
        }
    }

    pub fn not_found(operation: &'static str, entity: String, key: String) -> Self {
        Self::NotFound { operation, entity, key }
    }
}

pub trait WalletStoreReader {
    // Key manager
    fn key_manager_get_all(&mut self, branch: &str) -> Result<Vec<(u64, bool)>, WalletStorageError>;
    fn key_manager_get_active_index(&mut self, branch: &str) -> Result<u64, WalletStorageError>;
    fn key_manager_get_last_index(&mut self, branch: &str) -> Result<u64, WalletStorageError>;
    // Config
    fn config_get<T: serde::de::DeserializeOwned>(&mut self, key: &str) -> Result<Config<T>, WalletStorageError>;
    // Transactions
    fn transaction_get(&mut self, hash: FixedHash) -> Result<WalletTransaction, WalletStorageError>;
    fn transactions_fetch_all_by_status(
        &mut self,
        status: TransactionStatus,
    ) -> Result<Vec<WalletTransaction>, WalletStorageError>;
    // Substates
    fn substates_get(&mut self, address: &SubstateAddress) -> Result<SubstateModel, WalletStorageError>;
    fn substates_get_children(&mut self, parent: &SubstateAddress) -> Result<Vec<SubstateModel>, WalletStorageError>;
    // Accounts
    fn accounts_get(&mut self, address: &SubstateAddress) -> Result<Account, WalletStorageError>;
    fn accounts_get_many(&mut self, offset: u64, limit: u64) -> Result<Vec<Account>, WalletStorageError>;
    fn accounts_get_default(&mut self) -> Result<Account, WalletStorageError>;
    fn accounts_count(&mut self) -> Result<u64, WalletStorageError>;
    fn accounts_get_by_name(&mut self, name: &str) -> Result<Account, WalletStorageError>;
    fn accounts_get_by_vault(&mut self, vault_address: &SubstateAddress) -> Result<Account, WalletStorageError>;

    // Vaults
    fn vaults_get(&mut self, address: &SubstateAddress) -> Result<VaultModel, WalletStorageError>;
    fn vaults_get_by_resource(
        &mut self,
        account_addr: &SubstateAddress,
        resource_address: &ResourceAddress,
    ) -> Result<VaultModel, WalletStorageError>;
    fn vaults_get_by_account(&mut self, account_addr: &SubstateAddress) -> Result<Vec<VaultModel>, WalletStorageError>;

    // Outputs
    fn outputs_get_unspent_balance(&mut self, vault_address: &SubstateAddress) -> Result<u64, WalletStorageError>;
    fn outputs_get_locked_by_proof(
        &mut self,
        proof_id: ConfidentialProofId,
    ) -> Result<Vec<ConfidentialOutputModel>, WalletStorageError>;
    fn outputs_get_by_commitment(
        &mut self,
        commitment: &Commitment,
    ) -> Result<ConfidentialOutputModel, WalletStorageError>;

    fn outputs_get_by_account_and_status(
        &mut self,
        account_addr: &SubstateAddress,
        status: OutputStatus,
    ) -> Result<Vec<ConfidentialOutputModel>, WalletStorageError>;

    // Proofs
    fn proofs_get_by_transaction_hash(
        &mut self,
        transaction_hash: FixedHash,
    ) -> Result<ConfidentialProofId, WalletStorageError>;
}

pub trait WalletStoreWriter {
    fn commit(self) -> Result<(), WalletStorageError>;
    fn rollback(self) -> Result<(), WalletStorageError>;

    // JWT
    fn jwt_add_empty_token(&mut self) -> Result<u64, WalletStorageError>;
    fn jwt_store_decision(&mut self, id: u64, permissions_token: Option<String>) -> Result<(), WalletStorageError>;

    // Key manager
    fn key_manager_insert(&mut self, branch: &str, index: u64) -> Result<(), WalletStorageError>;
    fn key_manager_set_active_index(&mut self, branch: &str, index: u64) -> Result<(), WalletStorageError>;

    // Config
    fn config_set<T: serde::Serialize>(
        &mut self,
        key: &str,
        value: &T,
        is_encrypted: bool,
    ) -> Result<(), WalletStorageError>;

    // Transactions
    fn transactions_insert(&mut self, transaction: &Transaction, is_dry_run: bool) -> Result<(), WalletStorageError>;
    fn transactions_set_result_and_status(
        &mut self,
        hash: FixedHash,
        result: Option<&FinalizeResult>,
        transaction_failure: Option<&RejectReason>,
        final_fee: Option<Amount>,
        qcs: Option<&[QuorumCertificate<PublicKey>]>,
        new_status: TransactionStatus,
    ) -> Result<(), WalletStorageError>;

    // Substates
    fn substates_insert_parent(
        &mut self,
        tx_hash: FixedHash,
        address: VersionedSubstateAddress,
        module_name: String,
        template_addr: TemplateAddress,
    ) -> Result<(), WalletStorageError>;
    fn substates_insert_child(
        &mut self,
        tx_hash: FixedHash,
        parent: SubstateAddress,
        address: VersionedSubstateAddress,
    ) -> Result<(), WalletStorageError>;

    fn substates_remove(&mut self, substate: &VersionedSubstateAddress) -> Result<SubstateModel, WalletStorageError>;

    // Accounts
    fn accounts_set_default(&mut self, address: &SubstateAddress) -> Result<(), WalletStorageError>;
    fn accounts_insert(
        &mut self,
        account_name: &str,
        address: &SubstateAddress,
        owner_key_index: u64,
        is_default: bool,
    ) -> Result<(), WalletStorageError>;

    fn accounts_update(&mut self, address: &SubstateAddress, new_name: Option<&str>) -> Result<(), WalletStorageError>;

    // Vaults
    fn vaults_insert(&mut self, vault: VaultModel) -> Result<(), WalletStorageError>;
    fn vaults_update(
        &mut self,
        vault_address: &SubstateAddress,
        balance: Option<Amount>,
    ) -> Result<(), WalletStorageError>;

    // Confidential Outputs
    fn outputs_lock_smallest_amount(
        &mut self,
        vault_address: &SubstateAddress,
        locked_by_proof: ConfidentialProofId,
    ) -> Result<ConfidentialOutputModel, WalletStorageError>;
    fn outputs_insert(&mut self, output: ConfidentialOutputModel) -> Result<(), WalletStorageError>;
    /// Mark outputs as finalized
    fn outputs_finalize_by_proof_id(&mut self, proof_id: ConfidentialProofId) -> Result<(), WalletStorageError>;
    /// Release outputs that were locked and remove pending unconfirmed outputs for this proof
    fn outputs_release_by_proof_id(&mut self, proof_id: ConfidentialProofId) -> Result<(), WalletStorageError>;

    // Proofs
    fn proofs_insert(&mut self, vault_address: &SubstateAddress) -> Result<ConfidentialProofId, WalletStorageError>;
    fn proofs_delete(&mut self, proof_id: ConfidentialProofId) -> Result<(), WalletStorageError>;
    fn proofs_set_transaction_hash(
        &mut self,
        proof_id: ConfidentialProofId,
        transaction_hash: Hash,
    ) -> Result<(), WalletStorageError>;
}
