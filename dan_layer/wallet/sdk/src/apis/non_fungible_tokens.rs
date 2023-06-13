//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::{models::ResourceAddress, prelude::NonFungibleId};
use thiserror::Error;

use crate::{
    models::NonFungibleToken,
    storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter},
};

pub struct NonFungibleTokensApi<'a, TStore> {
    store: &'a TStore,
}

impl<'a, TStore> NonFungibleTokensApi<'a, TStore>
where
    TStore: WalletStore,
{
    pub fn new(store: &'a TStore) -> Self {
        Self { store }
    }

    pub fn store_new_nft(&self, non_fungible: &NonFungibleToken) -> Result<(), NonFungibleTokensApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.non_fungible_token_insert(non_fungible)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_non_fungible_token(&self, nft_id: NonFungibleId) -> Result<NonFungibleToken, NonFungibleTokensApiError> {
        let mut tx = self.store.create_read_tx()?;
        let non_fungible_token = tx.get_non_fungible_token(nft_id)?;
        Ok(non_fungible_token)
    }

    pub fn get_resource_address(&self, nft_id: NonFungibleId) -> Result<ResourceAddress, NonFungibleTokensApiError> {
        let mut tx = self.store.create_read_tx()?;
        let resource_address = tx.get_resource_address(nft_id)?;
        Ok(resource_address)
    }
}

#[derive(Debug, Error)]
pub enum NonFungibleTokensApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
}
