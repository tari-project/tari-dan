//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_engine_types::substate::SubstateAddress;

use crate::{
    models::Account,
    storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter},
};

pub struct AccountsApi<'a, TStore> {
    store: &'a TStore,
}

impl<'a, TStore: WalletStore> AccountsApi<'a, TStore> {
    pub fn new(store: &'a TStore) -> Self {
        Self { store }
    }

    pub fn add_account(
        &self,
        account_name: Option<&str>,
        account_address: &SubstateAddress,
        owner_key_index: u64,
    ) -> Result<(), AccountsApiError> {
        let mut tx = self.store.create_write_tx()?;
        let account_name = account_name
            .map(|s| s.to_string())
            .unwrap_or_else(|| account_address.to_string());
        if tx.accounts_get_by_name(&account_name).optional()?.is_some() {
            tx.rollback()?;
            return Err(AccountsApiError::AccountNameAlreadyExists { name: account_name });
        }
        tx.accounts_insert(&account_name, account_address, owner_key_index)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_many(&self, limit: u64) -> Result<Vec<Account>, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let accounts = tx.accounts_get_many(limit)?;
        Ok(accounts)
    }

    pub fn count(&self) -> Result<u64, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let count = tx.accounts_count()?;
        Ok(count)
    }

    pub fn get_account_by_name(&self, name: &str) -> Result<Account, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let account = tx.accounts_get_by_name(name)?;
        Ok(account)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AccountsApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Account name already exists: {name}")]
    AccountNameAlreadyExists { name: String },
}

impl IsNotFoundError for AccountsApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}
