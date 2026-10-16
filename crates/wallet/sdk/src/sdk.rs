//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Debug, sync::Arc};

use log::{info, warn};
use ootle_network::{Network, NetworkParseError};
use tari_common_types::seeds::{
    cipher_seed::CipherSeed,
    error::CipherError,
    mnemonic::{Mnemonic, MnemonicLanguage},
    seed_words::SeedWords,
};
use tari_crypto::tari_utilities::SafePassword;
use tari_ootle_common_types::{Epoch, optional::Optional};
use tari_ootle_wallet_crypto::StealthCryptoApi;
use zeroize::Zeroizing;

use crate::{
    apis::{
        accounts::AccountsApi,
        address_book::AddressBookApi,
        confidential_crypto::ConfidentialCryptoApi,
        confidential_outputs::ConfidentialOutputsApi,
        confidential_transfer::ConfidentialTransferApi,
        config::{ConfigApi, ConfigApiError, ConfigKey},
        events::EventsApi,
        key_manager::{KeyManagerApi, KeyManagerApiError},
        locks::LocksApi,
        non_fungible_tokens::NonFungibleTokensApi,
        password_manager::{PasswordManagerApi, PasswordManagerApiError},
        resources::ResourcesApi,
        signer::SignerApi,
        stealth_outputs::StealthOutputsApi,
        stealth_transfer::StealthTransferApi,
        substate::SubstatesApi,
        template::TemplateApi,
        transaction::TransactionApi,
        validator_set::{ValidatorSetApi, ValidatorSetShared},
        viewable_balance::ViewableBalanceApi,
    },
    cipher_seed::{CipherSeedRestore, WalletCipherSeed},
    local_key_store::LocalKeyStore,
    models::EpochBirthday,
    spec::WalletSdkSpec,
    storage::WalletStorageError,
};

const LOG_TARGET: &str = "wallet::sdk::api";

#[derive(Debug, Clone)]
pub struct WalletSdkConfig {
    pub network: Network,
    /// Encryption password for the wallet database.
    pub override_keyring_password: Option<SafePassword>,
    pub indexer_trust: IndexerTrust,
}

/// How far the wallet trusts what its indexer serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexerTrust {
    /// Commits a transaction's changes only once a proof shows the network committed them, and reads a substate only
    /// with a proof of its recent state, each verified against the validator sets the wallet walks to from its anchor.
    #[default]
    Verify,
    /// Takes the indexer at its word. For development against indexers that serve no proofs.
    Trust,
}

impl IndexerTrust {
    pub fn is_verify(self) -> bool {
        matches!(self, Self::Verify)
    }
}

pub struct WalletSdk<TSpec: WalletSdkSpec> {
    store: TSpec::Store,
    network_interface: TSpec::NetworkInterface,
    key_store: TSpec::KeyStore,
    config: WalletSdkConfig,
    epoch_birthday: EpochBirthday,
    validator_sets: Arc<ValidatorSetShared>,
}

impl<TSpec: WalletSdkSpec> WalletSdk<TSpec> {
    pub fn initialize(
        store: TSpec::Store,
        indexer: TSpec::NetworkInterface,
        key_store: TSpec::KeyStore,
        config: WalletSdkConfig,
        epoch_birthday: EpochBirthday,
    ) -> Result<Self, WalletSdkError> {
        Self::check_or_set_store_network(&store, config.network)?;

        Ok(Self {
            store,
            network_interface: indexer,
            key_store,
            config,
            epoch_birthday,
            validator_sets: Arc::default(),
        })
    }

    pub fn get_store_network(store: &TSpec::Store) -> Result<Option<Network>, WalletSdkError> {
        let config_api = ConfigApi::new(store);
        let network = config_api.get(ConfigKey::Network).optional()?;
        Ok(network)
    }

    fn check_or_set_store_network(store: &TSpec::Store, config_network: Network) -> Result<(), WalletSdkError> {
        if let Some(network) = Self::get_store_network(store)? {
            if config_network != network {
                return Err(WalletSdkError::InvariantError {
                    details: format!(
                        "Network mismatch. Config network is {:?} but database network is {:?}",
                        config_network, network
                    ),
                });
            }
        } else {
            ConfigApi::new(&store).set(ConfigKey::Network, config_network.as_key_str())?;
        }
        Ok(())
    }

    pub fn store(&self) -> &TSpec::Store {
        &self.store
    }

    pub fn config_api(&self) -> ConfigApi<'_, TSpec::Store> {
        ConfigApi::new(&self.store)
    }

    pub fn sdk_config(&self) -> &WalletSdkConfig {
        &self.config
    }

    pub fn network(&self) -> Network {
        self.config.network
    }

    pub fn get_network_interface(&self) -> &TSpec::NetworkInterface {
        &self.network_interface
    }

    pub fn locks_api(&self) -> LocksApi<'_, TSpec::Store> {
        LocksApi::new(&self.store)
    }

    pub fn event_api(&self) -> EventsApi<'_, TSpec::Store> {
        EventsApi::new(&self.store)
    }

    /// Returns the KeyManager API for the wallet.
    /// This key manager uses the configured key store to access key material.
    pub fn key_manager_api(&self) -> KeyManagerApi<'_, TSpec> {
        let network = self.config.network;
        KeyManagerApi::new(
            network,
            &self.store,
            &self.key_store,
            self.password_manager_api(),
            self.epoch_birthday,
        )
    }

    /// Returns the Signer API for the wallet. This API uses the configured key store.
    pub fn signer_api(&self) -> SignerApi<'_, TSpec> {
        SignerApi::new(self.key_manager_api())
    }

    pub(crate) fn password_manager_api(&self) -> PasswordManagerApi<'_, TSpec::Store> {
        PasswordManagerApi::new(self.config_api(), &self.config)
    }

    pub fn transaction_api(&self) -> TransactionApi<'_, TSpec::Store, TSpec::NetworkInterface> {
        TransactionApi::new(
            &self.store,
            &self.network_interface,
            self.validator_set_api(),
            self.config.indexer_trust,
        )
    }

    pub fn validator_set_api(&self) -> ValidatorSetApi<'_, TSpec::Store, TSpec::NetworkInterface> {
        ValidatorSetApi::new(
            &self.store,
            &self.network_interface,
            self.config.network,
            &self.validator_sets,
        )
    }

    pub fn substate_api(&self) -> SubstatesApi<'_, TSpec::Store, TSpec::NetworkInterface> {
        SubstatesApi::new(
            &self.store,
            &self.network_interface,
            self.validator_set_api(),
            self.config.indexer_trust,
        )
    }

    pub fn accounts_api(&self) -> AccountsApi<'_, TSpec> {
        AccountsApi::new(
            self.config.network,
            &self.store,
            self.substate_api(),
            self.key_manager_api(),
            self.epoch_birthday,
        )
    }

    pub fn resources_api(&self) -> ResourcesApi<'_, TSpec::Store> {
        ResourcesApi::new(&self.store)
    }

    pub fn confidential_crypto_api(&self) -> ConfidentialCryptoApi {
        ConfidentialCryptoApi::new()
    }

    pub fn confidential_outputs_api(&self) -> ConfidentialOutputsApi<'_, TSpec> {
        ConfidentialOutputsApi::new(&self.store, self.key_manager_api(), self.confidential_crypto_api())
    }

    pub fn confidential_transfer_api(&self) -> ConfidentialTransferApi<'_, TSpec> {
        ConfidentialTransferApi::new(
            self.key_manager_api(),
            self.accounts_api(),
            self.locks_api(),
            self.confidential_outputs_api(),
            self.substate_api(),
            self.transaction_api(),
            self.confidential_crypto_api(),
            self.config_api(),
        )
    }

    pub fn stealth_crypto_api(&self) -> StealthCryptoApi {
        StealthCryptoApi::new()
    }

    pub fn stealth_transfer_api(&self) -> StealthTransferApi<'_, TSpec> {
        StealthTransferApi::new(
            self.accounts_api(),
            self.stealth_outputs_api(),
            self.locks_api(),
            self.substate_api(),
            self.key_manager_api(),
            self.config_api(),
        )
    }

    pub fn stealth_outputs_api(&self) -> StealthOutputsApi<'_, TSpec> {
        StealthOutputsApi::new(
            self.store(),
            self.key_manager_api(),
            self.stealth_crypto_api(),
            self.config_api(),
        )
    }

    pub fn non_fungible_api(&self) -> NonFungibleTokensApi<'_, TSpec::Store> {
        NonFungibleTokensApi::new(&self.store)
    }

    pub fn address_book_api(&self) -> AddressBookApi<'_, TSpec::Store> {
        AddressBookApi::new(&self.store)
    }

    pub fn template_api(&self) -> TemplateApi<'_, TSpec::Store> {
        TemplateApi::new(&self.store)
    }

    pub fn viewable_balance_api(&self) -> ViewableBalanceApi {
        ViewableBalanceApi
    }

    pub fn calculate_birthday_epoch(&self) -> Epoch {
        self.epoch_birthday.calculate_current_epoch()
    }

    pub fn is_recovery_needed(&self) -> Result<bool, WalletSdkError> {
        let recovery_needed = self
            .config_api()
            .get::<bool>(ConfigKey::RecoveryNeeded)
            .optional()?
            .unwrap_or(false);
        Ok(recovery_needed)
    }
}

impl<TSpec> WalletSdk<TSpec>
where TSpec: WalletSdkSpec<KeyStore = LocalKeyStore>
{
    pub fn initialize_with_local_key_store(
        store: TSpec::Store,
        indexer: TSpec::NetworkInterface,
        config: WalletSdkConfig,
        epoch_birthday: EpochBirthday,
    ) -> Result<Self, WalletSdkError> {
        Self::check_or_set_store_network(&store, config.network)?;

        let cipher_seed = Self::load_cipher_seed(
            ConfigApi::new(&store),
            PasswordManagerApi::new(ConfigApi::new(&store), &config),
        )?
        .map(WalletCipherSeed::CipherSeed)
        .unwrap_or(WalletCipherSeed::None);

        Ok(Self {
            store,
            network_interface: indexer,
            key_store: LocalKeyStore::new(cipher_seed),
            config,
            epoch_birthday,
            validator_sets: Arc::default(),
        })
    }

    fn load_cipher_seed(
        config_api: ConfigApi<'_, TSpec::Store>,
        password_manager_api: PasswordManagerApi<'_, TSpec::Store>,
    ) -> Result<Option<Arc<CipherSeed>>, WalletSdkError> {
        let Some(cipher_seed_encrypted) = config_api
            .get::<Zeroizing<Box<[u8]>>>(ConfigKey::CipherSeed)
            .optional()?
        else {
            // Cipher seed not found in DB. This is expected if the wallet has not been initialized yet.
            return Ok(None);
        };
        let password = password_manager_api.get_cipher_seed_password()?;
        let cipher_seed = CipherSeed::from_enciphered_bytes(&cipher_seed_encrypted, Some(password))?;
        Ok(Some(Arc::new(cipher_seed)))
    }

    fn create_cipher_seed(&mut self) -> Result<(), WalletSdkError> {
        let password = self.password_manager_api().create_cipher_seed_password()?;
        let cipher_seed = CipherSeed::random();
        let encrypted_cipher_seed = cipher_seed.encipher(Some(password))?;
        self.config_api().set(ConfigKey::CipherSeed, &encrypted_cipher_seed)?;
        self.key_store.set_cipher_seed(cipher_seed);
        Ok(())
    }

    /// Restores cipher seed from seed words, encrypts with a new random password (and saves to OS keychain)
    /// and replaces current cipher seed in the DB (to let every component use the new seed).
    fn restore_cipher_seed_from_seed_words(&mut self, seed_words: &SeedWords) -> Result<(), WalletSdkError> {
        let password = self.password_manager_api().create_cipher_seed_password()?;
        let cipher_seed = CipherSeed::from_mnemonic(seed_words, None)?;
        let encrypted_cipher_seed = cipher_seed.encipher(Some(password))?;
        self.config_api().set(ConfigKey::CipherSeed, &encrypted_cipher_seed)?;
        self.key_store.set_cipher_seed(cipher_seed);
        Ok(())
    }

    /// Retrieve the seed words from current cipher seed stored.
    pub fn load_seed_words(&mut self) -> Result<Option<SeedWords>, WalletSdkError> {
        let seed_words = self
            .key_store
            .cipher_seed()
            .map(|s| s.to_mnemonic(MnemonicLanguage::English, None))
            .transpose()?;
        Ok(seed_words)
    }

    /// Initializes the cipher seed for the wallet. Either creating a new cipher seed or recovering it from the provided
    /// seed words if provided and necessary. Returns true if the cipher seed was recovered from the seed words,
    /// otherwise false.
    pub fn initialize_cipher_seed(&mut self, restore: CipherSeedRestore<'_>) -> Result<bool, WalletSdkError> {
        match self.key_store.cipher_seed() {
            Some(_) => {
                if !restore.is_create_new() {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️  Wallet already initialized. Ignoring seed words provided for recovery.",
                    );
                }
                let requires_recovery = self.config_api().get(ConfigKey::RecoveryNeeded).optional()?;
                // This should have been set - it is an error if it is not
                requires_recovery.ok_or_else(|| WalletSdkError::InvariantError {
                    details: "Cipher seed already initialized but recovery_needed not set.".to_string(),
                })
            },
            None => match restore {
                CipherSeedRestore::CreateNewIfRequired => {
                    self.create_cipher_seed()?;
                    self.config_api().set(ConfigKey::RecoveryNeeded, &false)?;
                    Ok(false)
                },
                CipherSeedRestore::FromSeedWords(seed_words) => {
                    self.restore_cipher_seed_from_seed_words(seed_words)?;
                    info!(target: LOG_TARGET, "🔑 Successfully restored wallet seed key!");
                    self.config_api().set(ConfigKey::RecoveryNeeded, &true)?;
                    Ok(true)
                },
            },
        }
    }
}

impl<TSpec> Clone for WalletSdk<TSpec>
where
    TSpec: WalletSdkSpec,
    TSpec::Store: Clone,
    TSpec::NetworkInterface: Clone,
    TSpec::KeyStore: Clone,
{
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            network_interface: self.network_interface.clone(),
            key_store: self.key_store.clone(),
            config: self.config.clone(),
            epoch_birthday: self.epoch_birthday,
            validator_sets: self.validator_sets.clone(),
        }
    }
}

impl<TSpec: WalletSdkSpec> Debug for WalletSdk<TSpec> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalletSdk")
            .field("config", &self.config)
            .field("epoch_birthday", &self.epoch_birthday)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WalletSdkError {
    #[error("Wallet storage error: {0}")]
    WalletStorageError(#[from] WalletStorageError),
    #[error("Config API error: {0}")]
    ConfigApiError(#[from] ConfigApiError),
    #[error("Key manager error: {0}")]
    KeyManager(#[from] KeyManagerApiError),
    #[error("Cipher error: {0}")]
    CipherError(#[from] CipherError),
    #[error("Password manager error: {0}")]
    PasswordManagerError(#[from] PasswordManagerApiError),
    #[error(transparent)]
    NetworkParseError(#[from] NetworkParseError),
    #[error("Invariant error: {details}. This indicates a bug in the code.")]
    InvariantError { details: String },
}
