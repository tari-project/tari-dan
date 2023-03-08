//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use tari_crypto::tari_utilities::SafePassword;
use tari_dan_common_types::optional::Optional;
use tari_key_manager::cipher_seed::CipherSeed;

use crate::{
    apis::{
        accounts::AccountsApi,
        confidential_crypto::ConfidentialCryptoApi,
        confidential_outputs::ConfidentialOutputsApi,
        config::{ConfigApi, ConfigApiError, ConfigKey},
        key_manager::KeyManagerApi,
        substate::SubstatesApi,
        transaction::TransactionApi,
    },
    storage::{WalletStorageError, WalletStore},
};

#[derive(Debug, Clone)]
pub struct WalletSdkConfig {
    /// Encryption password for the wallet database. NOTE: Not yet implemented, this field is ignored
    pub password: Option<SafePassword>,
    pub validator_node_jrpc_endpoint: String,
}

#[derive(Debug, Clone)]
pub struct DanWalletSdk<TStore> {
    store: TStore,
    config: WalletSdkConfig,
    cipher_seed: Arc<CipherSeed>,
}

impl<TStore> DanWalletSdk<TStore> {}

impl<TStore: WalletStore> DanWalletSdk<TStore> {
    pub fn initialize(store: TStore, config: WalletSdkConfig) -> Result<Self, WalletSdkError> {
        let cipher_seed = Self::get_or_create_cipher_seed(&store)?;

        Ok(Self {
            store,
            config,
            cipher_seed: Arc::new(cipher_seed),
        })
    }

    pub fn config_api(&self) -> ConfigApi<'_, TStore> {
        ConfigApi::new(&self.store)
    }

    pub fn key_manager_api(&self) -> KeyManagerApi<'_, TStore> {
        KeyManagerApi::new(&self.store, &self.cipher_seed)
    }

    pub fn transaction_api(&self) -> TransactionApi<'_, TStore> {
        TransactionApi::new(&self.store, &self.config.validator_node_jrpc_endpoint)
    }

    pub fn substate_api(&self) -> SubstatesApi<'_, TStore> {
        SubstatesApi::new(&self.store)
    }

    pub fn accounts_api(&self) -> AccountsApi<'_, TStore> {
        AccountsApi::new(&self.store)
    }

    pub fn confidential_crypto_api(&self) -> ConfidentialCryptoApi {
        ConfidentialCryptoApi::new()
    }

    pub fn confidential_outputs_api(&self) -> ConfidentialOutputsApi<'_, TStore> {
        ConfidentialOutputsApi::new(&self.store, self.key_manager_api())
    }

    fn get_or_create_cipher_seed(store: &TStore) -> Result<CipherSeed, WalletSdkError> {
        let config_api = ConfigApi::new(store);
        let maybe_cipher_seed = config_api.get(ConfigKey::CipherSeed).optional()?;

        match maybe_cipher_seed {
            Some(cipher_seed) => Ok(cipher_seed),
            None => {
                let cipher_seed = CipherSeed::new();
                config_api.set(ConfigKey::CipherSeed, &cipher_seed, true)?;
                Ok(cipher_seed)
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WalletSdkError {
    #[error("Wallet storage error: {0}")]
    WalletStorageError(#[from] WalletStorageError),
    #[error("Config API error: {0}")]
    ConfigApiError(#[from] ConfigApiError),
}
