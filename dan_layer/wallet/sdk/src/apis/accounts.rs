//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::{
    models::{Amount, ResourceAddress},
    prelude::ResourceType,
};

use crate::{
    models::{Account, VaultModel},
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
        is_default: bool,
    ) -> Result<(), AccountsApiError> {
        let mut tx = self.store.create_write_tx()?;
        let account_name = account_name
            .map(|s| s.to_string())
            .unwrap_or_else(|| account_address.to_string());
        if tx.accounts_get_by_name(&account_name).optional()?.is_some() {
            tx.rollback()?;
            return Err(AccountsApiError::AccountNameAlreadyExists { name: account_name });
        }
        tx.accounts_insert(&account_name, account_address, owner_key_index, is_default)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_many(&self, offset: u64, limit: u64) -> Result<Vec<Account>, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let accounts = tx.accounts_get_many(offset, limit)?;
        Ok(accounts)
    }

    pub fn count(&self) -> Result<u64, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let count = tx.accounts_count()?;
        Ok(count)
    }

    // pub fn get_account_by_name_or_default(&self, name: Option<&str>) -> Result<Account, AccountsApiError> {
    //     let mut tx = self.store.create_read_tx()?;
    //     let account = match name {
    //         Some(name) => tx.accounts_get_by_name(name)?,
    //         None => tx.accounts_get_default()?,
    //     };
    //     Ok(account)
    // }

    pub fn get_default(&self) -> Result<Account, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let account = tx.accounts_get_default()?;
        Ok(account)
    }

    pub fn get_account_by_name(&self, name: &str) -> Result<Account, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let account = tx.accounts_get_by_name(name)?;
        Ok(account)
    }

    pub fn update_vault_balance(
        &self,
        vault_address: &SubstateAddress,
        new_balance: Amount,
    ) -> Result<(), AccountsApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.vaults_update(vault_address, Some(new_balance))?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_account_by_address(&self, address: &SubstateAddress) -> Result<Account, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let account = tx.accounts_get(address)?;
        Ok(account)
    }

    pub fn get_account_or_default(&self, address: Option<&SubstateAddress>) -> Result<Account, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        if let Some(address) = address {
            let account = tx.accounts_get(address)?;
            return Ok(account);
        }
        let account = tx.accounts_get_default()?;
        Ok(account)
    }

    pub fn get_by_vault(&self, vault_addr: &&SubstateAddress) -> Result<Account, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let account = tx.accounts_get_by_vault(vault_addr)?;
        Ok(account)
    }

    pub fn get_vault_by_resource(
        &self,
        account_addr: &SubstateAddress,
        resource_addr: &ResourceAddress,
    ) -> Result<VaultModel, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let vault = tx.vaults_get_by_resource(account_addr, resource_addr)?;
        Ok(vault)
    }

    pub fn get_vault(&self, vault_addr: &&SubstateAddress) -> Result<VaultModel, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let vault = tx.vaults_get(vault_addr)?;
        Ok(vault)
    }

    pub fn has_vault(&self, vault_addr: &SubstateAddress) -> Result<bool, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        // TODO: consider optimising
        let exists = tx.vaults_get(vault_addr).optional()?.is_some();
        Ok(exists)
    }

    pub fn set_default_account(&self, account_addr: &SubstateAddress) -> Result<(), AccountsApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.accounts_set_default(account_addr)?;
        tx.commit()?;
        Ok(())
    }

    pub fn add_vault(
        &self,
        account_address: SubstateAddress,
        vault_address: SubstateAddress,
        resource_address: ResourceAddress,
        resource_type: ResourceType,
        token_symbol: Option<String>,
    ) -> Result<(), AccountsApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.vaults_insert(VaultModel {
            account_address,
            address: vault_address,
            resource_address,
            resource_type,
            balance: Amount::zero(),
            token_symbol,
        })?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_account_by_vault(&self, vault_addr: &&SubstateAddress) -> Result<Account, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let account = tx.accounts_get_by_vault(vault_addr)?;
        Ok(account)
    }

    pub fn get_vaults_by_account(&self, account: &SubstateAddress) -> Result<Vec<VaultModel>, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let vaults = tx.vaults_get_by_account(account)?;
        Ok(vaults)
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
