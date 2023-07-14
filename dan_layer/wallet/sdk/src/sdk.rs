//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{sync::Arc, time::Duration};

use tari_crypto::tari_utilities::SafePassword;
use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_key_manager::cipher_seed::CipherSeed;

use crate::{
    apis::{
        accounts::AccountsApi,
        confidential_crypto::ConfidentialCryptoApi,
        confidential_outputs::ConfidentialOutputsApi,
        config::{ConfigApi, ConfigApiError, ConfigKey},
        jwt::JwtApi,
        key_manager::KeyManagerApi,
        non_fungible_tokens::NonFungibleTokensApi,
        substate::SubstatesApi,
        transaction::TransactionApi,
    },
    network::WalletNetworkInterface,
    storage::{WalletStorageError, WalletStore},
};

#[derive(Debug, Clone)]
pub struct WalletSdkConfig {
    /// Encryption password for the wallet database. NOTE: Not yet implemented, this field is ignored
    pub password: Option<SafePassword>,
    pub indexer_jrpc_endpoint: String,
    pub jwt_expiry: Duration,
    pub jwt_secret_key: String,
}

#[derive(Debug, Clone)]
pub struct DanWalletSdk<TStore, TNetworkInterface> {
    store: TStore,
    network_interface: TNetworkInterface,
    config: WalletSdkConfig,
    cipher_seed: Arc<CipherSeed>,
}

impl<TStore, TNetworkInterface> DanWalletSdk<TStore, TNetworkInterface>
where
    TStore: WalletStore,
    TNetworkInterface: WalletNetworkInterface,
    TNetworkInterface::Error: IsNotFoundError,
{
    pub fn initialize(
        store: TStore,
        indexer: TNetworkInterface,
        config: WalletSdkConfig,
    ) -> Result<Self, WalletSdkError> {
        let cipher_seed = Self::get_or_create_cipher_seed(&store)?;

        Ok(Self {
            store,
            network_interface: indexer,
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

    pub fn transaction_api(&self) -> TransactionApi<'_, TStore, TNetworkInterface> {
        TransactionApi::new(&self.store, &self.network_interface)
    }

    pub fn substate_api(&self) -> SubstatesApi<'_, TStore, TNetworkInterface> {
        SubstatesApi::new(&self.store, &self.network_interface)
    }

    pub fn accounts_api(&self) -> AccountsApi<'_, TStore> {
        AccountsApi::new(&self.store)
    }

    pub fn confidential_crypto_api(&self) -> ConfidentialCryptoApi {
        ConfidentialCryptoApi::new()
    }

    pub fn jwt_api(&self) -> JwtApi<'_, TStore> {
        JwtApi::new(&self.store, self.config.jwt_expiry, self.config.jwt_secret_key.clone())
    }

    pub fn confidential_outputs_api(&self) -> ConfidentialOutputsApi<'_, TStore> {
        ConfidentialOutputsApi::new(
            &self.store,
            self.key_manager_api(),
            self.accounts_api(),
            self.confidential_crypto_api(),
        )
    }

    pub fn non_fungible_api(&self) -> NonFungibleTokensApi<'_, TStore> {
        NonFungibleTokensApi::new(&self.store)
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
