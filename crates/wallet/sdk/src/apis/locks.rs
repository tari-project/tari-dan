//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_engine_types::substate::SubstateDiff;
use tari_ootle_common_types::optional::IsNotFoundError;
use tari_ootle_transaction::TransactionId;
use tari_template_lib::types::{Amount, VaultId};

use crate::{
    models::{LockOwner, WalletLockDropGuard, WalletLockId},
    storage::{ReadableWalletStore, WalletStorageError, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};

#[derive(Clone)]
pub struct LocksApi<'a, TStore> {
    store: &'a TStore,
}

impl<'a, TStore> LocksApi<'a, TStore> {
    pub(crate) fn new(store: &'a TStore) -> Self {
        Self { store }
    }
}

impl<'a, TStore: WriteableWalletStore> LocksApi<'a, TStore> {
    pub fn create_lock(&self) -> Result<WalletLockDropGuard<'a, TStore>, LocksApiError> {
        let lock_id = self
            .store
            .with_write_tx(|tx| tx.locks_create(None, LockOwner::Wallet))?;
        Ok(WalletLockDropGuard::new(lock_id, self.store))
    }

    pub fn create_lock_with_timeout(
        &self,
        timeout: Duration,
    ) -> Result<WalletLockDropGuard<'a, TStore>, LocksApiError> {
        self.create_owned_lock_with_timeout(timeout, LockOwner::Wallet)
    }

    /// A lock whose id is handed back to `owner`, who alone may later name it
    /// in a transaction request.
    pub fn create_owned_lock_with_timeout(
        &self,
        timeout: Duration,
        owner: LockOwner,
    ) -> Result<WalletLockDropGuard<'a, TStore>, LocksApiError> {
        let lock_id = self.store.with_write_tx(|tx| tx.locks_create(Some(timeout), owner))?;
        Ok(WalletLockDropGuard::new(lock_id, self.store))
    }

    pub fn release_lock(&self, lock_id: WalletLockId) -> Result<(), LocksApiError> {
        self.store.with_write_tx(|tx| tx.locks_release(lock_id))?;
        Ok(())
    }

    pub fn finalize_lock(&self, lock_id: WalletLockId, diff: &SubstateDiff) -> Result<(), LocksApiError> {
        self.store
            .with_write_tx(|tx| tx.locks_unlock_finalized(lock_id, diff))?;
        Ok(())
    }

    pub fn lock_funds_in_vault<A: Into<Amount>>(
        &self,
        lock_id: WalletLockId,
        vault_id: &VaultId,
        amount_to_lock: A,
    ) -> Result<(), LocksApiError> {
        self.store
            .with_write_tx(|tx| tx.vaults_lock_revealed_funds(lock_id, vault_id, amount_to_lock.into()))?;

        Ok(())
    }

    pub fn clear_stale_locks(&self) -> Result<usize, LocksApiError> {
        let num = self.store.with_write_tx(|tx| tx.locks_release_stale())?;
        Ok(num)
    }
}

impl<TStore: ReadableWalletStore> LocksApi<'_, TStore> {
    pub fn get_lock_by_transaction_id(&self, transaction_id: TransactionId) -> Result<WalletLockId, LocksApiError> {
        let lock_id = self
            .store
            .with_read_tx(|tx| tx.locks_get_by_transaction_id(transaction_id))?;
        Ok(lock_id)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum LocksApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
}

impl IsNotFoundError for LocksApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, LocksApiError::StoreError(e) if e.is_not_found_error())
    }
}
