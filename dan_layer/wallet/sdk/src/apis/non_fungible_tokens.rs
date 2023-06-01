//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::ResourceAddress;
use thiserror::Error;

use crate::storage::{WalletStorageError, WalletStore, WalletStoreWriter};

use super::accounts::{AccountsApi, AccountsApiError};

pub struct NonFungibleTokenApi<'a, TStore> {
    store: &'a TStore,
    accounts_api: AccountsApi<'a, TStore>,
}

impl<'a, TStore: WalletStore> NonFungibleTokenApi<'a, TStore> {
    pub fn new(store: &'a TStore, accounts_api: AccountsApi<'a, TStore>) -> Self {
        Self { store, accounts_api }
    }

    pub fn create_new_nft_collection(&self) {}

    pub fn get_all_non_fungible_tokens(
        &self,
        account_name: &str,
    ) -> Result<Vec<ResourceAddress>, NonFungibleTokenApiError> {
        let tx = self.store.create_read_tx()?;
        let accounts_api = self.accounts_api.get_account_by_name(&account_name)?;
        Ok(vec![])
    }
}

#[derive(Debug, Error)]
pub enum NonFungibleTokenApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("AccountsApi error: {0}")]
    AccountsApiError(#[from] AccountsApiError),
}
