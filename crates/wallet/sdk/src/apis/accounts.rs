//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use ootle_byte_type::{ConvertFromByteType, FromByteType, ToByteType};
use tari_crypto::{
    keys::PublicKey as _,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::hex::Hex,
};
use tari_engine_types::{
    SubstateVersion,
    component::derive_component_address_from_public_key,
    indexed_value::IndexedWellKnownTypes,
};
use tari_ootle_address::{Network, RistrettoOotleAddress};
use tari_ootle_common_types::{
    Epoch,
    optional::{IsNotFoundError, Optional},
    substate_type::SubstateType,
};
use tari_ootle_transaction::TransactionId;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::types::{
    Amount,
    ComponentAddress,
    ResourceAddress,
    ResourceType,
    VaultId,
    constants::TARI_TOKEN,
    crypto::RistrettoPublicKeyBytes,
};

use crate::{
    apis::{
        confidential_transfer::{ConfidentialTransferApiError, ResolvedAccountDetails},
        config::{ConfigApi, ConfigApiError, ConfigKey},
        key_manager::{KeyManagerApi, KeyManagerApiError},
        substate::{SubstatesApi, ValidatorScanResult},
    },
    models::{
        Account,
        AccountUpdate,
        AccountWithAddress,
        BalanceChangePage,
        BalanceChangeSnapshot,
        BalanceChangeSource,
        BalanceChangeSourceType,
        EpochBirthday,
        KeyId,
        KeyIdOrPublicKey,
        KeyType,
        VaultBalance,
        VaultModel,
        WalletOotleAddressWithKeyIds,
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

const LOG_TARGET: &str = "tari::ootle::wallet_sdk::apis::accounts";

pub struct AccountsApi<'a, TSpec: WalletSdkSpec> {
    network: Network,
    store: &'a TSpec::Store,
    substates_api: SubstatesApi<'a, TSpec::Store, TSpec::NetworkInterface>,
    key_manager_api: KeyManagerApi<'a, TSpec>,
    epoch_birthday: EpochBirthday,
}

pub fn derive_account_address_from_public_key(public_key: &RistrettoPublicKeyBytes) -> ComponentAddress {
    derive_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, public_key)
}

impl<'a, TSpec: WalletSdkSpec> AccountsApi<'a, TSpec> {
    pub fn new(
        network: Network,
        store: &'a TSpec::Store,
        substates_api: SubstatesApi<'a, TSpec::Store, TSpec::NetworkInterface>,
        key_manager_api: KeyManagerApi<'a, TSpec>,
        epoch_birthday: EpochBirthday,
    ) -> Self {
        Self {
            network,
            store,
            substates_api,
            key_manager_api,
            epoch_birthday,
        }
    }

    pub fn derive_account_address_from_public_key(&self, public_key: &RistrettoPublicKeyBytes) -> ComponentAddress {
        derive_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, public_key)
    }

    /// Seed recovery may have checked this key index and deleted it as unused. Payments sent to it after recovery
    /// are only found if the scan starts from the seed birthday.
    fn birthday_epoch_for_new_account(&self, owner_key_id: &KeyId) -> Result<Epoch, AccountsApiError> {
        let max_probed_index = ConfigApi::new(self.store)
            .get::<u64>(ConfigKey::RecoveryMaxProbedKeyIndex)
            .optional()?;
        let was_probed = owner_key_id
            .derived_index()
            .zip(max_probed_index)
            .is_some_and(|(index, max_probed)| index <= max_probed);
        if was_probed {
            return Ok(self.key_manager_api.get_cipher_seed_birthday_epoch()?);
        }
        Ok(self.epoch_birthday.calculate_current_epoch())
    }

    pub fn create_account(
        &self,
        account_name: Option<&str>,
        is_default: bool,
        account_address: WalletOotleAddressWithKeyIds,
    ) -> Result<AccountWithAddress, AccountsApiError> {
        let account_public_key = account_address.address.account_key().to_byte_type();
        let account_component_address = self.derive_account_address_from_public_key(&account_public_key);

        let birthday_epoch = self.birthday_epoch_for_new_account(&account_address.owner_key_id)?;

        self.add_account(
            account_name,
            &account_component_address,
            account_address.view_only_key_id,
            account_address.owner_key_id,
            birthday_epoch,
            false,
            is_default,
        )?;

        Ok(AccountWithAddress {
            account: Account {
                name: account_name.map(String::from),
                component_address: account_component_address,
                view_only_key_id: account_address.view_only_key_id,
                owner_key_id: Some(account_address.owner_key_id),
                owner_public_key: account_public_key,
                birthday_epoch,
                is_confirmed_on_chain: false,
                is_default,
            },
            address: account_address.address.to_byte_type(),
        })
    }

    /// Adds an account that can be scanned and viewed but not spent from. The view secret key is stored as an
    /// imported key and the account has no owner key, so it is never the default account. Outputs are scanned from
    /// `birthday_epoch`, or the current epoch if not given. The view key must not be the view key of another
    /// account or an already imported key. The key and the account are stored together, or not at all.
    pub fn add_view_only_account(
        &self,
        account_name: Option<&str>,
        account_public_key: &RistrettoPublicKeyBytes,
        view_secret_key: &RistrettoSecretKey,
        birthday_epoch: Option<Epoch>,
    ) -> Result<AccountWithAddress, AccountsApiError> {
        let invalid = |reason: &str| AccountsApiError::InvalidViewOnlyAccount {
            reason: reason.to_string(),
        };
        let account_key = RistrettoPublicKey::convert_from_byte_type(account_public_key)
            .map_err(|_| invalid("account public key is not a valid public key"))?;
        if account_public_key.is_zero() {
            return Err(invalid("account public key is the identity point"));
        }
        if *view_secret_key == RistrettoSecretKey::default() {
            return Err(invalid("view secret key is zero"));
        }
        let view_public_key = RistrettoPublicKey::from_secret_key(view_secret_key);
        if view_public_key == account_key {
            return Err(invalid("view key is the account key"));
        }
        let current_epoch = self.epoch_birthday.calculate_current_epoch();
        if birthday_epoch.is_some_and(|epoch| epoch > current_epoch) {
            return Err(invalid("birthday epoch is after the current epoch"));
        }

        let account_address = self.derive_account_address_from_public_key(account_public_key);
        let birthday_epoch = birthday_epoch.unwrap_or(current_epoch);
        // Key store work happens before the write transaction, which holds the store's only connection
        let mut derived_view_keys = Vec::new();
        for account in self.store.with_read_tx(|tx| {
            let count = tx.accounts_count()?;
            tx.accounts_get_many(0, count as usize)
        })? {
            if let KeyId::Derived { key_branch, index } = account.view_only_key_id {
                let view_key = self.key_manager_api.derive_key(key_branch, index)?.to_public_key();
                derived_view_keys.push((account.component_address, view_key));
            }
        }
        let encrypted_view_key = self.key_manager_api.encrypt_imported_key(view_secret_key)?;

        // The checks, the key and the account are written in one transaction, so a failure leaves no imported key
        let view_key_id = self.store.with_write_tx(|tx| {
            if tx.accounts_get(&account_address).optional()?.is_some() {
                return Err(AccountsApiError::AccountAlreadyExists {
                    address: account_address,
                });
            }
            if tx
                .key_manager_get_imported_key_by_public_key(&view_public_key.to_hex())
                .optional()?
                .is_some()
            {
                return Err(AccountsApiError::ViewKeyAlreadyImported);
            }
            if let Some((address, _)) = derived_view_keys.iter().find(|(_, key)| *key == view_public_key) {
                return Err(AccountsApiError::ViewKeyAlreadyUsed { address: *address });
            }

            let view_key_id = self.key_manager_api.insert_imported_key(
                tx,
                &view_public_key.to_hex(),
                view_secret_key,
                &encrypted_view_key,
                KeyType::ViewOnly,
            )?;
            Self::insert_account(
                tx,
                account_name,
                &account_address,
                KeyId::imported(view_key_id),
                None,
                account_public_key,
                birthday_epoch,
                false,
                false,
            )?;
            Ok(view_key_id)
        })?;
        self.key_manager_api
            .cache_imported_key(view_key_id, view_secret_key, KeyType::ViewOnly);

        self.get_account_by_address(&account_address)
    }

    pub fn add_account<K: Into<KeyIdOrPublicKey>>(
        &self,
        account_name: Option<&str>,
        account_address: &ComponentAddress,
        view_only_key_id: KeyId,
        owner_key: K,
        birthday_epoch: Epoch,
        is_confirmed_on_chain: bool,
        is_default: bool,
    ) -> Result<(), AccountsApiError> {
        let (owner_pk, owner_key_id) = match owner_key.into() {
            KeyIdOrPublicKey::KeyId(key_id) => {
                let pk = self.key_manager_api.get_key(key_id)?.to_public_key().to_byte_type();
                (pk, Some(key_id))
            },
            KeyIdOrPublicKey::PublicKey(pk) => (pk, None),
        };
        self.store.with_write_tx(|tx| {
            Self::insert_account(
                tx,
                account_name,
                account_address,
                view_only_key_id,
                owner_key_id,
                &owner_pk,
                birthday_epoch,
                is_confirmed_on_chain,
                is_default,
            )
        })
    }

    fn insert_account(
        tx: &mut <TSpec::Store as WriteableWalletStore>::WriteTransaction<'_>,
        account_name: Option<&str>,
        account_address: &ComponentAddress,
        view_only_key_id: KeyId,
        owner_key_id: Option<KeyId>,
        owner_pk: &RistrettoPublicKeyBytes,
        birthday_epoch: Epoch,
        is_confirmed_on_chain: bool,
        is_default: bool,
    ) -> Result<(), AccountsApiError> {
        if is_default && owner_key_id.is_none() {
            return Err(AccountsApiError::ViewOnlyAccountCannotBeDefault);
        }
        if let Some(name) = account_name &&
            tx.accounts_get_by_name(name).optional()?.is_some()
        {
            return Err(AccountsApiError::AccountNameAlreadyExists { name: name.to_string() });
        }

        let mut associated_stealth_resources = HashSet::new();
        associated_stealth_resources.insert(TARI_TOKEN);

        tx.accounts_insert(
            account_name,
            account_address,
            view_only_key_id,
            owner_key_id,
            owner_pk,
            &associated_stealth_resources,
            birthday_epoch,
            is_confirmed_on_chain,
            is_default,
        )?;
        Ok(())
    }

    pub fn update_account(
        &self,
        account_address: &ComponentAddress,
        update: AccountUpdate<'_>,
    ) -> Result<(), AccountsApiError> {
        self.store.with_write_tx(|tx| {
            tx.accounts_update(account_address, update)?;
            Ok(())
        })
    }

    pub fn get_many(&self, offset: usize, limit: usize) -> Result<Vec<Account>, AccountsApiError> {
        let accounts = self.store.with_read_tx(|tx| tx.accounts_get_many(offset, limit))?;
        Ok(accounts)
    }

    pub fn count(&self) -> Result<u64, AccountsApiError> {
        let count = self.store.with_read_tx(|tx| tx.accounts_count())?;
        Ok(count)
    }

    pub fn has_default_account(&self) -> Result<bool, AccountsApiError> {
        let default_account = self.store.with_read_tx(|tx| tx.accounts_get_default().optional())?;
        Ok(default_account.is_some())
    }

    pub fn any_accounts_exist(&self) -> Result<bool, AccountsApiError> {
        self.count().map(|count| count > 0)
    }

    pub fn get_default(&self) -> Result<AccountWithAddress, AccountsApiError> {
        let account = self.store.with_read_tx(|tx| tx.accounts_get_default())?;
        let address = self.get_address_for_account(&account)?;
        Ok(AccountWithAddress {
            account,
            address: address.to_byte_type(),
        })
    }

    pub fn get_account_by_name(&self, name: &str) -> Result<AccountWithAddress, AccountsApiError> {
        let account = self.store.with_read_tx(|tx| tx.accounts_get_by_name(name))?;
        let address = self.get_address_for_account(&account)?;
        Ok(AccountWithAddress {
            account,
            address: address.to_byte_type(),
        })
    }

    pub fn update_vault_balance_and_record_change(
        &self,
        vault_address: VaultId,
        vault_version: SubstateVersion,
        revealed_balance: Amount,
        confidential_balance: Amount,
        source: BalanceChangeSource,
    ) -> Result<bool, AccountsApiError> {
        Ok(self.store.with_write_tx(|tx| -> Result<_, WalletStorageError> {
            let current_vault = tx.vaults_get(&vault_address)?;
            if current_vault.revealed_balance == revealed_balance &&
                current_vault.confidential_balance == confidential_balance
            {
                return Ok(false);
            }
            let recorded = tx.balance_changes_insert(
                BalanceChangeSnapshot {
                    account_address: current_vault.account_address,
                    vault_address: Some(current_vault.id),
                    vault_version: Some(vault_version),
                    resource_address: current_vault.resource_address,
                    token_symbol: current_vault.token_symbol,
                    divisibility: current_vault.divisibility,
                    revealed_before: current_vault.revealed_balance,
                    revealed_after: revealed_balance,
                    confidential_before: current_vault.confidential_balance,
                    confidential_after: confidential_balance,
                },
                source,
            )?;
            if !recorded && source.transaction_id().is_some() {
                return Ok(false);
            }
            tx.vaults_update(vault_address, vault_version, revealed_balance, confidential_balance)?;
            Ok(true)
        })?)
    }

    pub fn record_resource_balance_change(
        &self,
        account_address: ComponentAddress,
        resource_address: ResourceAddress,
        revealed_balance: Amount,
        confidential_balance: Amount,
        source: BalanceChangeSource,
    ) -> Result<bool, AccountsApiError> {
        Ok(self.store.with_write_tx(|tx| -> Result<_, WalletStorageError> {
            let maybe_vault = tx
                .vaults_get_by_resource(&account_address, &resource_address)
                .optional()?;
            let (vault_to_update, token_symbol, divisibility, revealed_before, confidential_before) =
                if let Some(vault) = maybe_vault {
                    (
                        Some((vault.id, vault.vault_version)),
                        vault.token_symbol,
                        vault.divisibility,
                        vault.revealed_balance,
                        vault.confidential_balance,
                    )
                } else {
                    let Some(resource) = tx.resources_get(&resource_address).optional()? else {
                        log::warn!(
                            target: LOG_TARGET,
                            "Skipping balance-change record for account {} resource {} because resource metadata is not cached",
                            account_address,
                            resource_address
                        );
                        return Ok(false);
                    };
                    let latest =
                        tx.balance_changes_get_latest_by_account_resource(&account_address, &resource_address)?;
                    (
                        None,
                        resource.resource.token_symbol().map(ToOwned::to_owned),
                        resource.resource.divisibility(),
                        latest.as_ref().map(|change| change.revealed_after).unwrap_or_default(),
                        latest
                            .as_ref()
                            .map(|change| change.confidential_after)
                            .unwrap_or_default(),
                    )
                };

            let recorded = tx.balance_changes_insert(
                BalanceChangeSnapshot {
                    account_address,
                    vault_address: None,
                    vault_version: None,
                    resource_address,
                    token_symbol,
                    divisibility,
                    revealed_before,
                    revealed_after: revealed_balance,
                    confidential_before,
                    confidential_after: confidential_balance,
                },
                source,
            )?;
            if recorded && let Some((vault_address, vault_version)) = vault_to_update {
                tx.vaults_update(vault_address, vault_version, revealed_balance, confidential_balance)?;
            }
            Ok(recorded)
        })?)
    }

    pub fn get_balance_changes(
        &self,
        account_address: &ComponentAddress,
        offset: usize,
        limit: usize,
        resource_address: Option<&ResourceAddress>,
        transaction_id: Option<&TransactionId>,
        source_type: Option<BalanceChangeSourceType>,
    ) -> Result<BalanceChangePage, AccountsApiError> {
        let page = self.store.with_read_tx(|tx| {
            tx.balance_changes_get_page_by_account(
                account_address,
                offset,
                limit,
                resource_address,
                transaction_id,
                source_type,
            )
        })?;
        Ok(page)
    }

    pub fn attribute_balance_change_to_transaction(
        &self,
        vault_id: &VaultId,
        vault_version: SubstateVersion,
        transaction_id: TransactionId,
    ) -> Result<bool, AccountsApiError> {
        let attributed = self
            .store
            .with_write_tx(|tx| tx.balance_changes_attribute_transaction(vault_id, vault_version, transaction_id))?;
        Ok(attributed)
    }

    pub fn get_vault_balance(&self, vault_address: &VaultId) -> Result<VaultBalance, AccountsApiError> {
        let vault = self.store.with_read_tx(|tx| tx.vaults_get(vault_address))?;
        Ok(VaultBalance {
            account: vault.account_address,
            confidential: vault.confidential_balance,
            revealed: vault.revealed_balance,
        })
    }

    pub fn exists_by_address(&self, address: &ComponentAddress) -> Result<bool, AccountsApiError> {
        Ok(self.get_account_by_address(address).optional()?.is_some())
    }

    pub fn get_account_by_address(&self, address: &ComponentAddress) -> Result<AccountWithAddress, AccountsApiError> {
        let account = self.store.with_read_tx(|tx| tx.accounts_get(address))?;
        let address = self.get_address_for_account(&account)?;
        Ok(AccountWithAddress {
            account,
            address: address.to_byte_type(),
        })
    }

    pub fn get_address_for_account(&self, account: &Account) -> Result<RistrettoOotleAddress, AccountsApiError> {
        let view_only_key = self
            .key_manager_api
            .get_public_key(account.view_only_key_id)?
            .public_key;
        let address = RistrettoOotleAddress::new(
            self.network,
            view_only_key,
            account
                .owner_public_key()
                .try_from_byte_type()
                .map_err(|e| WalletStorageError::DataInconsistent {
                    operation: "get_address_for_account",
                    details: format!("Failed to convert owner public key from byte type: {e}"),
                })?,
        );
        Ok(address)
    }

    pub fn associate_stealth_resource(
        &self,
        account_address: &ComponentAddress,
        stealth_resource_address: ResourceAddress,
    ) -> Result<(), AccountsApiError> {
        self.store
            .with_write_tx(|tx| tx.accounts_add_stealth_resource(account_address, stealth_resource_address))?;
        Ok(())
    }

    /// Returns true if the account has vaults, outputs, queued UTXOs, balance changes or transactions.
    pub fn has_activity(&self, address: &ComponentAddress) -> Result<bool, AccountsApiError> {
        let has_activity = self.store.with_read_tx(|tx| tx.accounts_has_activity(address))?;
        Ok(has_activity)
    }

    /// Deletes the account if it is not confirmed on chain and has no activity. If it was the default account, the
    /// first remaining account becomes the default. Returns true if the account was deleted.
    pub fn delete_if_unused(&self, address: &ComponentAddress) -> Result<bool, AccountsApiError> {
        self.store.with_write_tx(|tx| {
            let account = tx.accounts_get(address)?;
            if account.is_confirmed_on_chain || tx.accounts_has_activity(address)? {
                return Ok(false);
            }
            tx.accounts_delete(address)?;
            if account.is_default {
                let count = tx.accounts_count()?;
                let next_default = tx
                    .accounts_get_many(0, count as usize)?
                    .into_iter()
                    .find(|account| account.owner_key_id.is_some());
                if let Some(next_default) = next_default {
                    tx.accounts_set_default(next_default.component_address())?;
                }
            }
            Ok(true)
        })
    }

    pub fn get_associated_stealth_resources(
        &self,
        address: &ComponentAddress,
    ) -> Result<HashSet<ResourceAddress>, AccountsApiError> {
        let resources = self
            .store
            .with_read_tx(|tx| tx.accounts_get_associated_stealth_resources(address))?;
        Ok(resources)
    }

    pub fn get_account_by_public_key(
        &self,
        public_key: &RistrettoPublicKeyBytes,
    ) -> Result<AccountWithAddress, AccountsApiError> {
        let account_address = derive_account_address_from_public_key(public_key);
        let account = self.store.with_read_tx(|tx| tx.accounts_get(&account_address))?;
        let address = self.get_address_for_account(&account)?;
        Ok(AccountWithAddress {
            account,
            address: address.to_byte_type(),
        })
    }

    pub fn get_account_or_default(&self, address: Option<&ComponentAddress>) -> Result<Account, AccountsApiError> {
        self.store.with_read_tx(|tx| {
            if let Some(address) = address {
                let account = tx.accounts_get(address)?;
                return Ok(account);
            }
            let account = tx.accounts_get_default()?;
            Ok(account)
        })
    }

    pub fn get_by_vault(&self, vault_addr: &VaultId) -> Result<Account, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let account = tx.accounts_get_by_vault(vault_addr)?;
        Ok(account)
    }

    pub fn get_vault_by_resource(
        &self,
        account_addr: &ComponentAddress,
        resource_addr: &ResourceAddress,
    ) -> Result<VaultModel, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let vault = tx.vaults_get_by_resource(account_addr, resource_addr)?;
        Ok(vault)
    }

    pub fn get_vault(&self, vault_addr: &VaultId) -> Result<VaultModel, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let vault = tx.vaults_get(vault_addr)?;
        Ok(vault)
    }

    pub fn has_account(&self, addr: &ComponentAddress) -> Result<bool, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        // TODO: consider optimising
        let exists = tx.accounts_get(addr).optional()?.is_some();
        Ok(exists)
    }

    pub fn has_vault(&self, vault_id: &VaultId) -> Result<bool, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let exists = tx.vaults_exists(vault_id)?;
        Ok(exists)
    }

    pub fn rename_account(&self, account_addr: &ComponentAddress, new_name: &str) -> Result<(), AccountsApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.accounts_update(account_addr, AccountUpdate {
            name: Some(new_name),
            ..Default::default()
        })?;
        tx.commit()?;
        Ok(())
    }

    /// Makes the account the default account, which pays fees and signs, so it must have an owner key.
    pub fn set_default_account(&self, account_addr: &ComponentAddress) -> Result<(), AccountsApiError> {
        self.store.with_write_tx(|tx| {
            if tx.accounts_get(account_addr)?.owner_key_id.is_none() {
                return Err(AccountsApiError::ViewOnlyAccountCannotBeDefault);
            }
            tx.accounts_set_default(account_addr)?;
            Ok(())
        })
    }

    pub fn add_vault(
        &self,
        account_address: ComponentAddress,
        vault_address: VaultId,
        vault_version: SubstateVersion,
        resource_address: ResourceAddress,
        resource_type: ResourceType,
        token_symbol: Option<String>,
        divisibility: u8,
    ) -> Result<(), AccountsApiError> {
        let mut tx = self.store.create_write_tx()?;
        // A stealth resource's confidential balance is the account's UTXO sum, which may already have a
        // recorded history before the vault exists. Seed the vault with the last recorded balance so the
        // vault refresh states only what the current transaction changed, not the whole prior balance.
        let confidential_balance = if resource_type.is_stealth() {
            tx.balance_changes_get_latest_recorded(&account_address, &resource_address)?
                .map(|change| change.confidential_after)
                .unwrap_or_default()
        } else {
            Amount::zero()
        };
        tx.vaults_insert(VaultModel {
            account_address,
            id: vault_address,
            vault_version,
            resource_address,
            resource_type,
            revealed_balance: Amount::zero(),
            confidential_balance,
            locked_revealed_balance: Amount::zero(),
            token_symbol,
            divisibility,
        })?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_account_by_vault(&self, vault_addr: &&VaultId) -> Result<Account, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let account = tx.accounts_get_by_vault(vault_addr)?;
        Ok(account)
    }

    pub fn get_vault_ids_by_account(&self, account: &ComponentAddress) -> Result<Vec<VaultId>, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let vaults = tx.vaults_get_ids_by_account(account)?;
        Ok(vaults)
    }

    pub fn get_vaults_by_account(&self, account: &ComponentAddress) -> Result<Vec<VaultModel>, AccountsApiError> {
        let mut tx = self.store.create_read_tx()?;
        let vaults = tx.vaults_get_by_account(account)?;
        Ok(vaults)
    }
}

impl<'a, TSpec> AccountsApi<'a, TSpec>
where TSpec: WalletSdkSpec
{
    pub async fn resolve_account_by_public_key(
        &self,
        destination_pk: &RistrettoPublicKeyBytes,
    ) -> Result<ResolvedAccountDetails, ConfidentialTransferApiError> {
        let account_component = derive_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, destination_pk);
        // Is it an account we own?
        if let Some(vault_ids) = self.get_vault_ids_by_account(&account_component).optional()? {
            let account = self.get_account_by_address(&account_component)?;

            return Ok(ResolvedAccountDetails {
                address: account_component,
                vaults: vault_ids,
                exists_on_chain: account.is_confirmed_on_chain(),
            });
        }

        match self
            .substates_api
            .fetch_substate_from_network(&account_component.into(), None)
            .await
            .optional()?
        {
            Some(ValidatorScanResult {
                id: address, substate, ..
            }) => {
                let indexed_component = substate
                    .component()
                    .map(|c| IndexedWellKnownTypes::from_value(c.state()))
                    .transpose()
                    .map_err(|e| ConfidentialTransferApiError::UnexpectedIndexerResponse {
                        details: format!("Failed to extract vaults from account component: {e}"),
                    })?
                    .ok_or_else(|| ConfidentialTransferApiError::UnexpectedIndexerResponse {
                        details: format!(
                            "Expected indexer to return a component for account {} but it returned {} (value type: {})",
                            account_component,
                            address,
                            SubstateType::from(&substate)
                        ),
                    })?;
                Ok(ResolvedAccountDetails {
                    address: account_component,
                    vaults: indexed_component.vault_ids().to_vec(),
                    exists_on_chain: true,
                })
            },
            None => Ok(ResolvedAccountDetails {
                address: account_component,
                vaults: vec![],
                exists_on_chain: false,
            }),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AccountsApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Key manager API error: {0}")]
    KeyManagerApiError(#[from] KeyManagerApiError),
    #[error("Account name already exists: {name}")]
    AccountNameAlreadyExists { name: String },
    #[error("Account already exists: {address}")]
    AccountAlreadyExists { address: ComponentAddress },
    #[error("Invalid view-only account: {reason}")]
    InvalidViewOnlyAccount { reason: String },
    #[error("The view key is already used by account {address}")]
    ViewKeyAlreadyUsed { address: ComponentAddress },
    #[error("The view key is already an imported key")]
    ViewKeyAlreadyImported,
    #[error("A view-only account cannot be the default account")]
    ViewOnlyAccountCannotBeDefault,
    #[error("Config API error: {0}")]
    ConfigApiError(#[from] ConfigApiError),
}

impl AccountsApiError {
    pub fn is_name_exists_error(&self) -> bool {
        matches!(self, Self::AccountNameAlreadyExists { .. })
    }

    pub fn is_account_exists_error(&self) -> bool {
        matches!(self, Self::AccountAlreadyExists { .. })
    }

    pub fn is_invalid_view_only_account_error(&self) -> bool {
        matches!(self, Self::InvalidViewOnlyAccount { .. })
    }

    pub fn is_view_key_used_error(&self) -> bool {
        matches!(self, Self::ViewKeyAlreadyUsed { .. } | Self::ViewKeyAlreadyImported)
    }
}

impl IsNotFoundError for AccountsApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}
