//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::IsNotFoundError;
use tari_engine_types::substate::SubstateAddress;

use crate::{
    models::VersionedSubstateAddress,
    storage::{WalletStorageError, WalletStore, WalletStoreReader},
};

pub struct SubstatesApi<'a, TStore> {
    store: &'a TStore,
}

impl<'a, TStore: WalletStore> SubstatesApi<'a, TStore> {
    pub fn new(store: &'a TStore) -> Self {
        Self { store }
    }

    pub fn load_dependent_substates(
        &self,
        parents: &[SubstateAddress],
    ) -> Result<Vec<VersionedSubstateAddress>, SubstateApiError> {
        let mut substate_addresses = Vec::with_capacity(parents.len());
        let mut tx = self.store.create_read_tx()?;
        // TODO: Could be optimised, also perhaps we need to traverse all ancestors
        for parent_addr in parents {
            let parent = tx.substates_get(parent_addr)?;
            let children = tx.substates_get_children(parent_addr)?;
            substate_addresses.push(parent.address);
            substate_addresses.extend(children.into_iter().map(|s| s.address));
        }
        Ok(substate_addresses)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubstateApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
}

impl IsNotFoundError for SubstateApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}
