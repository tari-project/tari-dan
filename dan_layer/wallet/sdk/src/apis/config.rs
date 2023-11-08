//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{de::DeserializeOwned, Serialize};
use tari_dan_common_types::optional::IsNotFoundError;

use crate::storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter};

#[derive(Debug)]
pub struct ConfigApi<'a, TStore> {
    store: &'a TStore,
}

impl<'a, TStore: WalletStore> ConfigApi<'a, TStore> {
    pub fn new(store: &'a TStore) -> Self {
        Self { store }
    }

    pub fn get<T>(&self, key: ConfigKey) -> Result<T, ConfigApiError>
    where T: DeserializeOwned {
        let mut tx = self.store.create_read_tx()?;
        let record = tx.config_get(key.as_key_str())?;
        Ok(record.value)
    }

    pub fn set<T: Serialize>(&self, key: ConfigKey, value: &T, is_encrypted: bool) -> Result<(), ConfigApiError> {
        let mut tx = self.store.create_write_tx()?;
        // TODO: Actually encrypt if is_encrypted is true
        tx.config_set(key.as_key_str(), value, is_encrypted)?;
        tx.commit()?;
        Ok(())
    }
}

pub enum ConfigKey {
    CipherSeed,
    IndexerUrl,
}

impl ConfigKey {
    pub fn as_key_str(&self) -> &'static str {
        match self {
            ConfigKey::CipherSeed => "cipher_seed",
            ConfigKey::IndexerUrl => "indexer_url",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
}

impl IsNotFoundError for ConfigApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error())
    }
}
