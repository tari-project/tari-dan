//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::FromByteType;
use ootle_network::Network;
use tari_common_types::seeds::cipher_seed;
use tari_crypto::{
    keys::{PublicKey as _, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::{ByteArray, hex::Hex},
};
use tari_ootle_address::RistrettoOotleAddress;
use tari_ootle_common_types::{
    Epoch,
    optional::{IsNotFoundError, Optional},
    signature::{SignatureOutput, TRANSACTION_SIGNING_CONTEXT},
};
use tari_ootle_transaction::Signable;
use tari_ootle_wallet_crypto::{
    StealthCryptoApi,
    encryption::{CipherError, decrypt_with_password, encrypt_with_password},
};

use crate::{
    apis::password_manager::{PasswordManagerApi, PasswordManagerApiError},
    key_managers::WalletKeyStore,
    models::{
        DerivedKeyId,
        DerivedKeyIndex,
        DerivedKeyPair,
        DerivedWalletKey,
        EpochBirthday,
        ImportedKeyId,
        ImportedWalletKey,
        KeyBranch,
        KeyId,
        KeyType,
        StealthUtxoSpendKeyId,
        WalletKeyRecord,
        WalletOotleAddressWithKeyIds,
        WalletPublicKey,
        WalletSecretKey,
    },
    spec::WalletSdkSpec,
    storage::{
        CommittableStore,
        ReadableWalletStore,
        WalletStorageError,
        WalletStoreReader,
        WalletStoreWriter,
        WriteableWalletStore,
    },
};

pub struct KeyManagerApi<'a, TSpec: WalletSdkSpec> {
    network: Network,
    store: &'a TSpec::Store,
    key_store: &'a TSpec::KeyStore,
    password_manager: PasswordManagerApi<'a, TSpec::Store>,
    crypto_api: StealthCryptoApi,
    epoch_birthday: EpochBirthday,
}

impl<'a, TSpec: WalletSdkSpec> KeyManagerApi<'a, TSpec> {
    pub(crate) fn new(
        network: Network,
        store: &'a TSpec::Store,
        key_store: &'a TSpec::KeyStore,
        password_manager: PasswordManagerApi<'a, TSpec::Store>,
        epoch_birthday: EpochBirthday,
    ) -> Self {
        Self {
            network,
            store,
            key_store,
            password_manager,
            crypto_api: StealthCryptoApi::new(),
            epoch_birthday,
        }
    }

    pub(crate) fn key_store(&self) -> &'a TSpec::KeyStore {
        self.key_store
    }

    pub fn get_all_derived_keys(&self, branch: KeyBranch) -> Result<Vec<WalletKeyRecord>, KeyManagerApiError> {
        let all_keys = self.store.with_read_tx(|tx| tx.key_manager_get_all(branch.as_str()))?;
        let mut keys = Vec::with_capacity(all_keys.len());

        for (index, active) in all_keys {
            let key = self
                .key_store
                .derive_secret(branch.as_str(), index)
                .map_err(KeyManagerApiError::key_store_error)?;
            let pk = RistrettoPublicKey::from_secret_key(&key);
            keys.push(WalletKeyRecord {
                key_id: KeyId::derived(branch, index),
                public_key: pk,
                secret_key: key,
                is_active: active,
            });
        }
        Ok(keys)
    }

    pub fn get_imported_key(&self, id: ImportedKeyId) -> Result<ImportedWalletKey, KeyManagerApiError> {
        let password = self.password_manager.get_cipher_seed_password()?;
        let (_ty, encrypted) = self.store.with_read_tx(|tx| tx.key_manager_get_raw_imported_key(id))?;
        let decrypted = decrypt_with_password(&encrypted, password.reveal())?;
        let secret =
            RistrettoSecretKey::from_canonical_bytes(&decrypted).map_err(|e| WalletStorageError::DecodingError {
                operation: "get_imported_secret",
                item: "imported secret key",
                details: format!("Imported key at id {id} is non-canonical {e}"),
            })?;
        Ok(ImportedWalletKey {
            key: secret,
            import_id: id,
        })
    }

    pub fn import_key(
        &self,
        label: &str,
        secret_key: &RistrettoSecretKey,
        key_type: KeyType,
    ) -> Result<KeyId, KeyManagerApiError> {
        let password = self.password_manager.get_cipher_seed_password()?;
        let encrypted_key = encrypt_with_password(secret_key.as_bytes(), password.reveal()).map_err(|e| {
            KeyManagerApiError::StoreError(WalletStorageError::EncryptionError {
                operation: "KeyManagerApi::import_key",
                details: format!("Failed to encrypt imported key: {}", e),
            })
        })?;
        let public_key = RistrettoPublicKey::from_secret_key(secret_key).to_hex();
        let id = self
            .store
            .with_write_tx(|tx| tx.key_manager_insert_imported_key(label, &public_key, &encrypted_key, key_type))?;
        Ok(KeyId::imported(id))
    }

    pub fn get_key(&self, key_id: KeyId) -> Result<WalletSecretKey, KeyManagerApiError> {
        match key_id {
            KeyId::Imported { local_key_id } => {
                let imported_key = self.get_imported_key(local_key_id)?;
                Ok(imported_key.into())
            },
            KeyId::Derived { key_branch, index } => {
                let derived_key = self.derive_key(key_branch, index)?;
                Ok(derived_key.into())
            },
        }
    }

    pub fn get_public_key<T: Into<KeyId>>(&self, key_id: T) -> Result<WalletPublicKey, KeyManagerApiError> {
        let key_id = key_id.into();
        match key_id {
            KeyId::Imported { local_key_id } => {
                // TODO: could be implemented without fetching the secret key, if we stored the public key in the DB
                let imported_key = self.get_imported_key(local_key_id)?;
                Ok(WalletPublicKey {
                    public_key: imported_key.to_public_key(),
                    key_id,
                })
            },
            KeyId::Derived { key_branch, index } => {
                let derived_key = self.derive_key(key_branch, index)?;
                Ok(WalletPublicKey {
                    public_key: derived_key.to_public_key(),
                    key_id,
                })
            },
        }
    }

    pub fn get_elgamal_encrypted_view_key(
        &self,
        index: DerivedKeyIndex,
    ) -> Result<DerivedWalletKey, KeyManagerApiError> {
        self.derive_key(KeyBranch::ElgamalEncryptionViewKey, index)
    }

    pub(crate) fn derive_key(
        &self,
        branch: KeyBranch,
        index: DerivedKeyIndex,
    ) -> Result<DerivedWalletKey, KeyManagerApiError> {
        let secret = self
            .key_store
            .derive_secret(branch.as_str(), index)
            .map_err(KeyManagerApiError::key_store_error)?;
        Ok(DerivedWalletKey {
            key: secret,
            derived_key_id: DerivedKeyId { branch, index },
        })
    }

    pub(crate) fn generate_stealth_owner_key(
        &self,
        key_id: &StealthUtxoSpendKeyId,
    ) -> Result<RistrettoSecretKey, KeyManagerApiError> {
        let public_nonce = key_id
            .public_nonce
            .try_from_byte_type()
            .map_err(|e| KeyManagerApiError::InvalidKeyId {
                details: format!("Failed to convert public nonce to RistrettoPublicKey: {}", e),
            })?;
        let account_key = self.get_key(key_id.account_key_id)?;
        let view_key = self.get_key(key_id.view_key_id)?;
        Ok(self.crypto_api.derive_stealth_owner_secret(
            self.network,
            account_key.secret(),
            view_key.secret(),
            &public_nonce,
        ))
    }

    pub fn derive_keypair(
        &self,
        branch: KeyBranch,
        key_index: DerivedKeyIndex,
    ) -> Result<DerivedKeyPair, KeyManagerApiError> {
        let derived_key = self.derive_key(branch, key_index)?;
        Ok(DerivedKeyPair {
            public_key: derived_key.to_public_key(),
            derived_key,
        })
    }

    pub fn derive_account_key(&self, index: DerivedKeyIndex) -> Result<DerivedWalletKey, KeyManagerApiError> {
        self.derive_key(KeyBranch::Account, index)
    }

    pub fn derive_account_address(
        &self,
        index: DerivedKeyIndex,
    ) -> Result<WalletOotleAddressWithKeyIds, KeyManagerApiError> {
        let key = self.derive_key(KeyBranch::Account, index)?;
        let view_only_key = self.derive_key(KeyBranch::ViewOnlyKey, index)?;
        Ok(WalletOotleAddressWithKeyIds {
            address: RistrettoOotleAddress {
                network: self.network,
                view_only_key: RistrettoPublicKey::from_secret_key(&view_only_key.key),
                account_key: RistrettoPublicKey::from_secret_key(&key.key),
                pay_ref: None,
            },
            view_only_key_id: view_only_key.as_key_id(),
            owner_key_id: (*key.derived_key_id()).into(),
        })
    }

    pub fn next_account_address(&self) -> Result<WalletOotleAddressWithKeyIds, KeyManagerApiError> {
        let key = self.next_key(KeyBranch::Account)?;
        self.derive_account_address(key.key_index())
    }

    pub fn derive_account_key_pair(&self, index: u64) -> Result<DerivedKeyPair, KeyManagerApiError> {
        let key = self.derive_account_key(index)?;
        let public_key = RistrettoPublicKey::from_secret_key(&key.key);
        Ok(DerivedKeyPair {
            public_key,
            derived_key: key,
        })
    }

    pub fn last_index(&self, branch: &str) -> Result<u64, KeyManagerApiError> {
        let mut tx = self.store.create_read_tx()?;
        Ok(tx.key_manager_get_last_index(branch).optional()?.unwrap_or(0))
    }

    /// Derives the next key in the specified branch, increments the index, and sets it as the active key.
    /// If the branch does not exist, it will be created with index 0 and the first key will be returned.
    /// TODO: if there is another active DB transaction this function will block until it can acquire it.
    pub fn next_key(&self, branch: KeyBranch) -> Result<DerivedWalletKey, KeyManagerApiError> {
        let next_key_id = self.next_derived_key_id(branch)?;
        let key = self.derive_key(branch, next_key_id.index())?;
        Ok(key)
    }

    /// Derives the next key in the specified branch, increments the index, and sets it as the active key.
    /// If the branch does not exist, it will be created with index 0 and the first key will be returned.
    /// TODO: if there is another active DB transaction this function will block until it can acquire it.
    pub fn next_public_key(&self, branch: KeyBranch) -> Result<WalletPublicKey, KeyManagerApiError> {
        let next_key_id = self.next_derived_key_id(branch)?;
        let key = self.derive_key(branch, next_key_id.index())?;
        Ok(WalletPublicKey {
            public_key: key.to_public_key(),
            key_id: key.as_key_id(),
        })
    }

    pub fn next_derived_key_id(&self, branch: KeyBranch) -> Result<DerivedKeyId, KeyManagerApiError> {
        let mut tx = self.store.create_write_tx()?;
        let next_index = tx
            .key_manager_get_last_index(branch.as_str())
            .optional()?
            .map(|i| i.checked_add(1).ok_or(KeyManagerApiError::KeyIndexExhausted))
            .transpose()?
            .unwrap_or(0);
        if matches!(branch, KeyBranch::Account) {
            // Ensure the view key branch is created if it doesn't exist
            tx.key_manager_insert_or_ignore(KeyBranch::ViewOnlyKey.as_str(), next_index)?;
        }
        tx.key_manager_insert_or_ignore(branch.as_str(), next_index)?;
        tx.commit()?;
        Ok(DerivedKeyId {
            branch,
            index: next_index,
        })
    }

    pub fn create_throwaway_nonce(&self) -> RistrettoSecretKey {
        RistrettoSecretKey::random(&mut rand::rng())
    }

    pub fn set_active_key<B: AsRef<str>>(&self, branch: B, index: u64) -> Result<(), KeyManagerApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.key_manager_set_active_index(branch.as_ref(), index)?;
        tx.commit()?;
        Ok(())
    }

    /// Resets the active key index to the provided index for the given branch.
    /// A subsequent call to next_key will return the key for index + 1.
    /// If the active key is after the provided index, no key will be active.
    pub fn reset_key_index_to<B: AsRef<str>>(&self, branch: B, index: u64) -> Result<(), KeyManagerApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.key_manager_reset_index(branch.as_ref(), index)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_active_key(&self, branch: KeyBranch) -> Result<DerivedWalletKey, KeyManagerApiError> {
        let key_index = self
            .store
            .with_read_tx(|tx| tx.key_manager_get_active_index(branch.as_str()))
            .optional()?
            .unwrap_or(0);
        self.derive_key(branch, key_index)
    }

    pub fn get_cipher_seed_birthday_epoch(&self) -> Result<Epoch, KeyManagerApiError> {
        let birthday = self
            .key_store
            .key_birthday()
            .map_err(KeyManagerApiError::key_store_error)?;
        let Some(birthday) = birthday else {
            return Ok(Epoch::zero());
        };

        let birthday = u64::from(birthday) * cipher_seed::SECONDS_PER_DAY;
        let epoch = self.epoch_birthday.calculate_epoch_rel_minotari(birthday);

        Ok(epoch)
    }

    pub fn sign_with_stealth_key<T, C>(
        &self,
        key_id: &StealthUtxoSpendKeyId,
        context: C,
        item: &T,
    ) -> Result<T::Signature, KeyManagerApiError>
    where
        T: Signable<C>,
        T::Signature: From<SignatureOutput>,
    {
        let stealth_secret = self.generate_stealth_owner_key(key_id)?;
        self.sign_with_explicit_key(&stealth_secret, context, item)
    }

    pub fn sign_with_context<T, C>(
        &self,
        key_id: KeyId,
        context: C,
        item: &T,
    ) -> Result<T::Signature, KeyManagerApiError>
    where
        T: Signable<C>,
        T::Signature: From<SignatureOutput>,
    {
        if !key_id.is_signing_key() {
            return Err(KeyManagerApiError::NotASigningKey { key_id });
        }
        match &key_id {
            // Use the key store implementation to sign with derived keys
            KeyId::Derived { key_branch, index } => {
                let output = self
                    .key_store()
                    .sign(key_branch.as_str(), *index, context, item)
                    .map_err(KeyManagerApiError::key_store_error)?;
                Ok(output.into())
            },
            _ => {
                let key = self.get_key(key_id)?;
                self.sign_with_explicit_key(key.secret(), context, item)
            },
        }
    }

    pub fn sign_with_explicit_key<T, C>(
        &self,
        secret_key: &RistrettoSecretKey,
        context: C,
        item: &T,
    ) -> Result<T::Signature, KeyManagerApiError>
    where
        T: Signable<C>,
        T::Signature: From<SignatureOutput>,
    {
        let signature = TRANSACTION_SIGNING_CONTEXT.sign(secret_key, item.to_signing_message(context).as_ref());
        let public_key = RistrettoPublicKey::from_secret_key(secret_key);
        Ok(SignatureOutput { public_key, signature }.into())
    }
}

impl<TSpec: WalletSdkSpec> Clone for KeyManagerApi<'_, TSpec> {
    fn clone(&self) -> Self {
        Self {
            network: self.network,
            store: self.store,
            key_store: self.key_store,
            password_manager: self.password_manager.clone(),
            crypto_api: self.crypto_api,
            epoch_birthday: self.epoch_birthday,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KeyManagerApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Key store error: {source}")]
    KeyStoreError {
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("Key {key_id} not found")]
    KeyNotFound { key_id: KeyId },
    #[error("Key index exhausted for branch")]
    KeyIndexExhausted,
    #[error("Password manager error: {0}")]
    PasswordManagerApiError(#[from] PasswordManagerApiError),
    #[error("Key manager is in read only mode")]
    ReadOnlyMode,
    #[error("Invalid key id: {details}")]
    InvalidKeyId { details: String },
    #[error("Key {key_id} is not a signing key: only account, transaction, nonce and imported keys sign")]
    NotASigningKey { key_id: KeyId },
    #[error("Cipher error: {0}")]
    CipherError(#[from] CipherError),
}

impl KeyManagerApiError {
    pub fn key_store_error<E: std::error::Error + Send + Sync + 'static>(source: E) -> Self {
        KeyManagerApiError::KeyStoreError {
            source: Box::new(source),
        }
    }
}

impl IsNotFoundError for KeyManagerApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, KeyManagerApiError::KeyNotFound { .. }) ||
            matches!(self, KeyManagerApiError::StoreError(e) if e.is_not_found_error())
    }
}
