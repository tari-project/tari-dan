//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::{
    models::{Amount, ResourceAddress},
    prelude::{NonFungibleId, ResourceType},
    resource::TOKEN_SYMBOL,
};
use thiserror::Error;

use crate::{
    models::{Account, NonFungibleToken, VaultModel},
    storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter},
};

use super::accounts::AccountsApi;

pub struct NonFungibleTokensApi<'a, TStore> {
    store: &'a TStore,
    accounts_api: AccountsApi<'a, TStore>,
}

impl<'a, TStore> NonFungibleTokensApi<'a, TStore>
where
    TStore: WalletStore,
{
    pub fn new(store: &'a TStore, accounts_api: AccountsApi<'a, TStore>) -> Self {
        Self { store, accounts_api }
    }

    pub fn store_new_nft(
        &mut self,
        resource_address: ResourceAddress,
        non_fungible: NonFungibleToken,
    ) -> Result<(), NonFungibleTokensApiError> {
        let mut tx = self.store.create_write_tx()?;
        let nft_id = non_fungible.nft_id;
        let metadata = non_fungible.metadata;
        tx.store_non_fungible_token(
            nft_id,
            resource_address,
            metadata,
            non_fungible.token_symbol,
            &non_fungible.account_address,
        )?;
        Ok(())
    }

    pub fn get_non_fungible_token(
        &mut self,
        nft_id: NonFungibleId,
    ) -> Result<NonFungibleToken, NonFungibleTokensApiError> {
        let mut tx = self.store.create_read_tx()?;
        let non_fungible_token = tx.get_non_fungible_token(nft_id)?;
        Ok(non_fungible_token)
    }
}

#[derive(Debug, Error)]
pub enum NonFungibleTokensApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
}
