//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::PublicKey;
use tari_crypto::keys::PublicKey as PublicKeyTrait;
//
use tari_crypto::{hash::blake2::Blake256, ristretto::RistrettoSecretKey};
use tari_dan_common_types::optional::Optional;
use tari_key_manager::{
    cipher_seed::CipherSeed,
    key_manager::{DerivedKey, KeyManager},
};

use crate::storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter};

pub type WalletKeyManager = KeyManager<RistrettoSecretKey, Blake256>;

// TODO: Think these branches should be internal to the SDK
pub const TRANSACTION_BRANCH: &str = "transactions";
pub const OUTPUT_BRANCH: &str = "outputs";

pub struct KeyManagerApi<'a, TStore> {
    store: &'a TStore,
    cipher_seed: &'a CipherSeed,
}

impl<'a, TStore: WalletStore> KeyManagerApi<'a, TStore> {
    pub(crate) fn new(store: &'a TStore, cipher_seed: &'a CipherSeed) -> Self {
        Self { store, cipher_seed }
    }

    pub fn get_or_create_initial(&self, branch: &str) -> Result<(), KeyManagerApiError> {
        let mut tx = self.store.create_write_tx()?;
        if tx.key_manager_get_active_index(branch).optional()?.is_none() {
            tx.key_manager_set_active_index(branch, 0)?;
            tx.commit()?;
        } else {
            tx.rollback()?;
        }
        Ok(())
    }

    pub fn get_all_keys(&self, branch: &str) -> Result<Vec<(u64, PublicKey, bool)>, KeyManagerApiError> {
        let mut tx = self.store.create_read_tx()?;
        let all_keys = tx.key_manager_get_all(branch)?;
        let mut keys = Vec::with_capacity(all_keys.len());
        for (index, active) in all_keys {
            let km = self.get_key_manager(branch, index);
            let key = km
                .derive_key(index)
                .map_err(tari_key_manager::error::KeyManagerError::ByteArrayError)?;
            let pk = PublicKey::from_secret_key(&key.k);
            keys.push((index, pk, active));
        }
        Ok(keys)
    }

    pub fn derive_key(&self, branch: &str, index: u64) -> Result<DerivedKey<RistrettoSecretKey>, KeyManagerApiError> {
        let km = self.get_or_create_key_manager(branch)?;
        let key = km
                .derive_key(index)
                // TODO: Key manager shouldn't return other errors
                .map_err(tari_key_manager::error::KeyManagerError::ByteArrayError)?;
        Ok(key)
    }

    pub fn next_key(&self, branch: &str) -> Result<DerivedKey<RistrettoSecretKey>, KeyManagerApiError> {
        self.modify_key_manager_with(branch, |km| {
            let key = km
                .next_key()
                // TODO: Key manager shouldn't return other errors
                .map_err(tari_key_manager::error::KeyManagerError::ByteArrayError)?;
            Ok(key)
        })
    }

    pub fn set_active_key(&self, branch: &str, index: u64) -> Result<(), KeyManagerApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.key_manager_set_active_index(branch, index)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_active_key(&self, branch: &str) -> Result<(u64, DerivedKey<RistrettoSecretKey>), KeyManagerApiError> {
        let index = self
            .store
            .with_read_tx(|tx| tx.key_manager_get_active_index(branch))
            .optional()?
            .unwrap_or(0);
        Ok((index, self.derive_key(branch, index)?))
    }

    pub fn get_key_or_active(
        &self,
        branch: &str,
        maybe_index: Option<u64>,
    ) -> Result<(u64, DerivedKey<RistrettoSecretKey>), KeyManagerApiError> {
        match maybe_index {
            Some(index) => Ok((index, self.derive_key(branch, index)?)),
            None => self.get_active_key(branch),
        }
    }

    pub fn get_public_key(&self, branch: &str, key_index: Option<u64>) -> Result<PublicKey, KeyManagerApiError> {
        let (_, key) = self.get_key_or_active(branch, key_index)?;
        Ok(PublicKey::from_secret_key(&key.k))
    }

    fn get_or_create_key_manager(&self, branch: &str) -> Result<WalletKeyManager, KeyManagerApiError> {
        let mut tx = self.store.create_write_tx()?;
        let index = match tx.key_manager_get_active_index(branch).optional()? {
            Some(index) => {
                tx.rollback()?;
                index
            },
            None => {
                tx.key_manager_set_active_index(branch, 0)?;
                tx.commit()?;
                0
            },
        };
        Ok(self.get_key_manager(branch, index))
    }

    fn get_key_manager(&self, branch: &str, index: u64) -> WalletKeyManager {
        KeyManager::from(self.cipher_seed.clone(), branch.to_string(), index)
    }

    fn modify_key_manager_with<R, F: FnOnce(&mut WalletKeyManager) -> Result<R, KeyManagerApiError>>(
        &self,
        branch: &str,
        f: F,
    ) -> Result<R, KeyManagerApiError> {
        let mut tx = self.store.create_write_tx()?;
        let index = tx.key_manager_get_active_index(branch).optional()?.unwrap_or(0);
        let mut key_manager = KeyManager::from(self.cipher_seed.clone(), branch.to_string(), index);
        let ret = f(&mut key_manager)?;
        tx.key_manager_set_active_index(&key_manager.branch_seed, key_manager.key_index())?;
        tx.commit()?;
        Ok(ret)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KeyManagerApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Key manager error: {0}")]
    KeyManagerError(#[from] tari_key_manager::error::KeyManagerError),
}
