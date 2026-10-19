// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use log::*;
use ootle_byte_type::ToByteType;
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey};
use tari_engine_types::component::derive_component_address_from_public_key;
use tari_ootle_common_types::{
    Epoch,
    displayable::Displayable,
    optional::{IsNotFoundError, Optional},
    response_status::TransactionStatusResponseError,
    substate_type::SubstateType,
};
use tari_ootle_wallet_sdk::{
    WalletSdk,
    WalletSdkSpec,
    apis::config::ConfigKey,
    models::{DerivedWalletKey, KeyBranch, KeyId},
    network::WalletNetworkInterface,
};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib_types::ComponentAddress;
use tokio::time;

use crate::{
    account_monitor::AccountMonitorHandle,
    account_recovery::AccountRecoveryError,
    utxo_scanner::UtxoScannerHandle,
};

const LOG_TARGET: &str = "tari::ootle_wallet_daemon::resource_scanner";

const MAX_ATTEMPTS_PER_KEY: u32 = 5;
const RETRY_DELAY: Duration = Duration::from_secs(1);

/// Scans through all the substates to find related resources to current wallet.
pub struct AccountRecoveryService<TSpec: WalletSdkSpec> {
    wallet_sdk: WalletSdk<TSpec>,
    account_monitor_handle: AccountMonitorHandle,
    utxo_scanner_handle: UtxoScannerHandle,
    abandon_after_not_found: usize,
    cipher_seed_birthday_epoch: Epoch,
}

impl<TSpec> AccountRecoveryService<TSpec>
where
    TSpec: WalletSdkSpec,
    <TSpec::NetworkInterface as WalletNetworkInterface>::Error: IsNotFoundError + TransactionStatusResponseError,
{
    pub fn new(
        wallet_sdk: WalletSdk<TSpec>,
        account_monitor_handle: AccountMonitorHandle,
        utxo_scanner_handle: UtxoScannerHandle,
        abandon_after_not_found: usize,
        cipher_seed_birthday_epoch: Epoch,
    ) -> Self {
        Self {
            wallet_sdk,
            account_monitor_handle,
            utxo_scanner_handle,
            abandon_after_not_found,
            cipher_seed_birthday_epoch,
        }
    }

    pub async fn scan(self) {
        info!(target: LOG_TARGET, "Waiting for indexer to be ready...");

        // wait for indexer to be ready
        loop {
            match self.wallet_sdk.get_network_interface().wait_until_ready().await {
                Ok(()) => {
                    break;
                },
                Err(error) => {
                    warn!(target: LOG_TARGET, "Indexer is not ready yet: {error:?}");
                },
            }
            time::sleep(Duration::from_secs(5)).await;
        }

        info!(target: LOG_TARGET, "🔑 Attempting to recover accounts...");

        // if we don't find 10 accounts in a row just stop scanning
        let key_manager_api = self.wallet_sdk.key_manager_api();
        let mut not_found_accounts_count = 0;
        let mut found_accounts_count = 0;
        let mut last_found_key = None;
        let mut unused_accounts = Vec::new();
        // Every run starts from the first key, so a run that was interrupted is redone in full. Keys are derived
        // without advancing the key manager's index, which is set once the scan is complete.
        let mut key_index = 0;
        loop {
            let key = match key_manager_api.derive_account_key(key_index) {
                Ok(key) => key,
                Err(err) => {
                    error!(target: LOG_TARGET, "Error deriving key {key_index}: {err}. Scanning failed...");
                    return;
                },
            };
            info!(target: LOG_TARGET, "🔍️ Attempting to recover account with key index {}", key_index);
            match self.try_recover_account_with_retries(&key).await {
                Some(RecoveredAccount::Found) => {
                    last_found_key = Some(key_index);
                    info!(target: LOG_TARGET, "✅ Account with key index {} found!", key_index);
                    not_found_accounts_count = 0;
                    found_accounts_count += 1;
                },
                Some(RecoveredAccount::Unused(address)) => {
                    unused_accounts.push((key_index, address));
                    not_found_accounts_count += 1;
                },
                None => {
                    error!(
                        target: LOG_TARGET,
                        "❌ Could not scan the account at key index {key_index}. Recovery will run again when the wallet restarts"
                    );
                    return;
                },
            }
            if not_found_accounts_count == self.abandon_after_not_found {
                info!(
                    target: LOG_TARGET,
                    "No accounts found for {} consecutive keys. Assuming that we are done", self.abandon_after_not_found
                );
                break;
            }
            key_index += 1;
        }
        let max_probed_key = key_index;

        let active_key_index = last_found_key.unwrap_or(0);

        // The account at the active key index is kept so that a wallet with nothing to recover still has its first
        // account.
        for (_, address) in unused_accounts.iter().filter(|(index, _)| *index != active_key_index) {
            match self.wallet_sdk.accounts_api().delete_if_unused(address) {
                Ok(true) => info!(target: LOG_TARGET, "🗑️ Removed unused account {address}"),
                Ok(false) => info!(target: LOG_TARGET, "Keeping account {address} because it has activity"),
                Err(err) => warn!(target: LOG_TARGET, "⚠️ Error removing unused account {address}: {err}"),
            }
        }

        if let Err(err) = self
            .wallet_sdk
            .config_api()
            .set(ConfigKey::RecoveryMaxProbedKeyIndex, &max_probed_key)
        {
            error!(target: LOG_TARGET, "Error setting the highest recovered key index: {err}");
        }

        info!(target: LOG_TARGET, "Setting active key to {}", active_key_index);
        if let Err(err) = key_manager_api.reset_key_index_to(KeyBranch::Account, active_key_index) {
            error!(target: LOG_TARGET, "Error setting active key: {err}");
        }
        if let Err(err) = key_manager_api.set_active_key(KeyBranch::Account, active_key_index) {
            error!(target: LOG_TARGET, "Error setting active key: {err}");
        }

        // Set a flag to indicate that the wallet has completed recovery
        if let Err(err) = self.wallet_sdk.config_api().set(ConfigKey::RecoveryNeeded, &false) {
            error!(target: LOG_TARGET, "Error setting recovery needed flag: {err}");
        }

        info!(target: LOG_TARGET, "✅ Scanning accounts finished! {found_accounts_count} owned account(s) found!");
    }

    /// Returns None if the account could not be scanned after several attempts. The scan cannot skip the key: the
    /// key may hold the only UTXOs that keep the scan going, and its account row must not be left in place of an
    /// account the user creates later.
    async fn try_recover_account_with_retries(&self, key: &DerivedWalletKey) -> Option<RecoveredAccount> {
        for attempt in 1..=MAX_ATTEMPTS_PER_KEY {
            match self.try_recover_account(key).await {
                Ok(recovered) => return Some(recovered),
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Error scanning the account at key index {} (attempt {attempt}/{MAX_ATTEMPTS_PER_KEY}): {err}",
                        key.key_index()
                    );
                    if attempt < MAX_ATTEMPTS_PER_KEY {
                        time::sleep(RETRY_DELAY * attempt).await;
                    }
                },
            }
        }
        None
    }

    /// Adds the account derived from the key and scans it for vaults and stealth UTXOs. The account is found if it
    /// exists on chain or has ever received a stealth UTXO.
    async fn try_recover_account(&self, key: &DerivedWalletKey) -> Result<RecoveredAccount, AccountRecoveryError> {
        let public_key = RistrettoPublicKey::from_secret_key(&key.key).to_byte_type();
        // Derive the account address from the public key
        let account_addr = derive_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, &public_key);

        // Attempt to fetch the account substate
        // TODO: perf we should batch these requests e.g. query 10 accounts in one go
        let result = self
            .wallet_sdk
            .substate_api()
            .fetch_substate_from_network(&account_addr.into(), None)
            .await
            .optional()
            .map_err(|e| AccountRecoveryError::NetworkInterfaceError { details: e.to_string() })?;

        let is_on_chain = match result {
            None => {
                info!(target: LOG_TARGET, "🔑 Account {} not found on chain. It may have stealth UTXOs owned by its key", account_addr);
                false
            },
            Some(result) => {
                let component =
                    result
                        .substate
                        .as_component()
                        .ok_or_else(|| AccountRecoveryError::InvalidResponse {
                            details: format!(
                                "Expected component substate for address {account_addr}, got {}",
                                SubstateType::from(&result.substate)
                            ),
                        })?;

                if component.owner_public_key().is_none() {
                    warn!(target: LOG_TARGET, "⚠️ Account {} has no owner key. This wallet may not be able to use this account", account_addr);
                };

                if component.owner_public_key().is_some_and(|pk| *pk != public_key) {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Account {} has a different owner key {} than the one derived from the seed key {}. This wallet may not be able to use this account",
                        account_addr,
                        component.owner_public_key().display(),
                        public_key
                    );
                };

                info!(
                    target: LOG_TARGET,
                    "🔑 Adding account {} with owner key {} and key index {}",
                    account_addr,
                    component.owner_public_key().display(),
                    key.key_index()
                );
                true
            },
        };

        let accounts_api = self.wallet_sdk.accounts_api();
        // A previous recovery run that did not finish may have added the account already.
        if !accounts_api.exists_by_address(&account_addr)? {
            accounts_api.add_account(
                Some(format!("recovered-account-{}", key.key_index()).as_str()),
                &account_addr,
                KeyId::derived(KeyBranch::ViewOnlyKey, key.key_index()),
                key.as_key_id(),
                // We use the cipher seed birthday as the account birthday since that is simpler than attempting to
                // fetch the creation epoch for each account.
                self.cipher_seed_birthday_epoch,
                is_on_chain,
                // if this is the first account, set it as the default
                key.key_index() == 0,
            )?;
        }

        // Update vaults, nfts etc
        self.account_monitor_handle
            .refresh_account_for_recovery(account_addr)
            .await?;

        let mut num_potential_recoveries = 0;
        for resource_address in accounts_api.get_associated_stealth_resources(&account_addr)? {
            let stats = self.utxo_scanner_handle.scan(account_addr, resource_address).await?;
            num_potential_recoveries += stats.num_potential_recoveries;
        }

        if is_on_chain || num_potential_recoveries > 0 || accounts_api.has_activity(&account_addr)? {
            Ok(RecoveredAccount::Found)
        } else {
            Ok(RecoveredAccount::Unused(account_addr))
        }
    }
}

enum RecoveredAccount {
    /// The account exists on chain or has received stealth UTXOs.
    Found,
    /// Nothing was ever sent to the account.
    Unused(ComponentAddress),
}
