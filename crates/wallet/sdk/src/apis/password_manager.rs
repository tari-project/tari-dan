//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_network::NetworkParseError;
use passwords::PasswordGenerator;
use rand::Rng;
use tari_crypto::tari_utilities::SafePassword;
use tari_ootle_common_types::optional::{IsNotFoundError, Optional};

use crate::{
    Network,
    WalletSdkConfig,
    apis::config::{ConfigApi, ConfigApiError, ConfigKey},
    storage::WalletStore,
};

const KEYRING_ENTRIES_SERVICE: &str = "tari-ootle-wallet";
const CIPHER_SEED_PASSWORD_KEYRING_ENTRY_NAME: &str = "cipher-seed-password";

#[derive(Clone)]
pub struct PasswordManagerApi<'a, TStore> {
    override_keyring_password: Option<&'a SafePassword>,
    config_api: ConfigApi<'a, TStore>,
    network: Network,
}

impl<'a, TStore: WalletStore> PasswordManagerApi<'a, TStore> {
    pub(crate) fn new(config_api: ConfigApi<'a, TStore>, sdk_config: &'a WalletSdkConfig) -> Self {
        Self {
            config_api,
            override_keyring_password: sdk_config.override_keyring_password.as_ref(),
            network: sdk_config.network,
        }
    }

    pub fn get_cipher_seed_password(&self) -> Result<SafePassword, PasswordManagerApiError> {
        if let Some(password) = self.override_keyring_password {
            return Ok(password.clone());
        }

        let key = self.config_api.get::<String>(ConfigKey::KeyringPasswordEntryKey)?;
        let entry = self.get_cipher_seed_password_keyring_entry(&key)?;
        // If get_password fails with NoEntry, it means that the password is not set in the keyring i.e. IsNotFoundError
        // will return true which is what we want.
        let password = entry.get_password()?;
        Ok(SafePassword::from(password))
    }

    pub fn create_cipher_seed_password(&mut self) -> Result<SafePassword, PasswordManagerApiError> {
        if let Some(password) = self.override_keyring_password {
            // If we are overriding the keyring password, we don't need to set it in the keyring.
            // This is because the password is already set in the config.
            return Ok(password.clone());
        }

        let key = match self
            .config_api
            .get::<String>(ConfigKey::KeyringPasswordEntryKey)
            .optional()?
        {
            Some(key) => key,
            None => {
                // If the key is not set, we generate a new key and set it in the config.
                // The nonce is used to differentiate between different password entries in the keyring when running
                // multiple instances of the wallet on the same network. This nonce is generated once per wallet
                // database.
                let nonce = generate_password_entry_key_nonce();
                let key = format!("{}-{}-{}", CIPHER_SEED_PASSWORD_KEYRING_ENTRY_NAME, self.network, nonce);
                self.config_api.set(ConfigKey::KeyringPasswordEntryKey, &key)?;
                key
            },
        };

        let str_password = generate_password()?;
        let entry = self.get_cipher_seed_password_keyring_entry(&key)?;
        entry.set_password(&str_password)?;

        Ok(SafePassword::from(str_password))
    }

    fn get_cipher_seed_password_keyring_entry(
        &self,
        key: &str,
    ) -> Result<keyring_core::Entry, PasswordManagerApiError> {
        let result = keyring_core::Entry::new(KEYRING_ENTRIES_SERVICE, key);

        match result {
            Ok(entry) => Ok(entry),
            // NoDefaultStore means the binary did not install a credential store before using the SDK
            // (see e.g. tari_ootle_walletd::init_os_keyring_store). NoEntry from Entry::new can mask other
            // access errors. Both mean "the keyring is unusable", not "the password is not set", so we must
            // not let IsNotFoundError be true for them.
            Err(keyring_core::Error::NoEntry | keyring_core::Error::NoDefaultStore) => {
                Err(PasswordManagerApiError::FailedToAccessKeyRing)
            },
            Err(err) => Err(err.into()),
        }
    }
}

fn generate_password_entry_key_nonce() -> u64 {
    rand::rng().next_u64()
}

/// Generate a new random password.
fn generate_password() -> Result<String, PasswordManagerApiError> {
    let pg = PasswordGenerator {
        length: 256,
        numbers: true,
        lowercase_letters: true,
        uppercase_letters: true,
        symbols: false,
        spaces: false,
        exclude_similar_characters: false,
        strict: true,
    };
    let generated_password = pg
        .generate_one()
        .map_err(|error| PasswordManagerApiError::PasswordGeneration(error.to_string()))?;

    Ok(generated_password)
}

#[derive(Debug, thiserror::Error)]
pub enum PasswordManagerApiError {
    #[error("Config API error: {0}")]
    ConfigApiError(#[from] ConfigApiError),
    #[error("OS Keyring error: {0}")]
    KeyRing(#[from] keyring_core::Error),
    #[error("Failed to generate password for cipher seed: {0}")]
    PasswordGeneration(String),
    #[error(
        "The OS keyring is not available on this device. Provide the seed encryption password with the \
         `override_keyring_password` config setting (`--override-keyring-password` for tari_ootle_walletd)."
    )]
    FailedToAccessKeyRing,
    #[error(transparent)]
    NetworkParseError(#[from] NetworkParseError),
}

impl IsNotFoundError for PasswordManagerApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::KeyRing(e) if matches!(e, keyring_core::Error::NoEntry))
    }
}
