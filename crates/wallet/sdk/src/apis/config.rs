//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{str::FromStr, sync::OnceLock};

use ootle_network::Network;
use serde::{Serialize, de::DeserializeOwned};
use tari_ootle_common_types::optional::IsNotFoundError;

use crate::storage::{CommittableStore, WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter};

#[derive(Debug, Clone)]
pub struct ConfigApi<'a, TStore> {
    store: &'a TStore,
    cached_network: OnceLock<Network>,
}

impl<'a, TStore: WalletStore> ConfigApi<'a, TStore> {
    pub fn new(store: &'a TStore) -> Self {
        Self {
            store,
            cached_network: OnceLock::new(),
        }
    }

    pub fn get_network(&self) -> Result<Network, ConfigApiError> {
        if let Some(network) = self.cached_network.get() {
            return Ok(*network);
        }
        let network = self.get::<String>(ConfigKey::Network)?;
        let network = Network::from_str(&network).map_err(|e| ConfigApiError::FailedToParseNetwork {
            string: network,
            details: e.to_string(),
        })?;
        self.cached_network
            .set(network)
            .expect("Network should only be set once");
        Ok(network)
    }

    pub fn get<T: DeserializeOwned>(&self, key: ConfigKey) -> Result<T, ConfigApiError> {
        let mut tx = self.store.create_read_tx()?;
        let record = tx.config_get(key.as_key_str())?;
        if record.is_encrypted {
            return Err(ConfigApiError::EncryptedItem { key });
        }
        Ok(record.value)
    }

    pub fn exists(&self, key: ConfigKey) -> Result<bool, ConfigApiError> {
        let mut tx = self.store.create_read_tx()?;
        let exists = tx.config_exists(key.as_key_str())?;
        Ok(exists)
    }

    pub fn set<T: Serialize + ?Sized>(&self, key: ConfigKey, value: &T) -> Result<(), ConfigApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.config_set(key.as_key_str(), value, false)?;
        tx.commit()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ConfigKey {
    /// The network the wallet is running on. type: String
    Network,
    /// The cipher seed used to encrypt the wallet. type: Vec<u8>
    CipherSeed,
    /// The indexer URLs the wallet may use. type: Vec<Url>, or a single Url as stored by earlier versions
    IndexerUrls,
    /// Indicates whether the wallet needs to be recovered. type: bool
    RecoveryNeeded,
    /// The keyring key that stored the decryption password
    KeyringPasswordEntryKey,
    /// Advanced UI features. type: AdvancedUiFeatures
    AdvancedUiFeatures,
    /// Accounts that have successfully claimed testnet faucet funds. type: Vec<String>
    ClaimedAccounts,
    /// The highest account key index that seed recovery checked. type: u64
    RecoveryMaxProbedKeyIndex,
    /// The latest validator set the wallet trusts. type: TrustedValidatorSet
    TrustedValidatorSet,
    /// When the wallet last brought its trusted validator set up to date, in seconds since the Unix epoch. type: u64
    ValidatorSetSyncedAt,
}

impl ConfigKey {
    pub fn as_key_str(&self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::CipherSeed => "cipher_seed",
            Self::IndexerUrls => "indexer_url",
            Self::RecoveryNeeded => "recovery_needed",
            Self::KeyringPasswordEntryKey => "keyring_password_entry_key",
            Self::AdvancedUiFeatures => "advanced_ui_features",
            Self::ClaimedAccounts => "claimed_accounts",
            Self::RecoveryMaxProbedKeyIndex => "recovery_max_probed_key_index",
            Self::TrustedValidatorSet => "trusted_validator_set",
            Self::ValidatorSetSyncedAt => "validator_set_synced_at",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Failed to parse network string '{string}': {details}")]
    FailedToParseNetwork { string: String, details: String },
    #[error("The requested item is encrypted and cannot be retrieved without decryption: {key:?}")]
    EncryptedItem { key: ConfigKey },
}

impl IsNotFoundError for ConfigApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error())
    }
}
