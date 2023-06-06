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

    pub fn store_new_nft(&self, non_fungible: NonFungibleToken) -> Result<(), NonFungibleTokensApiError> {
        let mut tx = self.store.create_write_tx()?;
        let nft_id = non_fungible.nft_id;
        let metadata = non_fungible.metadata;
        let resource_address = non_fungible.resource_address;
        tx.store_non_fungible_token(
            nft_id,
            resource_address,
            metadata,
            non_fungible.token_symbol,
            &non_fungible.account_address,
        )?;
        Ok(())
    }

    pub fn get_non_fungible_token(&self, nft_id: NonFungibleId) -> Result<NonFungibleToken, NonFungibleTokensApiError> {
        let mut tx = self.store.create_read_tx()?;
        let non_fungible_token = tx.get_non_fungible_token(nft_id)?;
        Ok(non_fungible_token)
    }

    pub fn get_resource_address(&self, token_symbol: String) -> Result<ResourceAddress, NonFungibleTokensApiError> {
        let mut tx = self.store.create_read_tx()?;
        let resource_address = tx.get_resource_address(token_symbol)?;
        Ok(resource_address)
    }
}

#[derive(Debug, Error)]
pub enum NonFungibleTokensApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
}
