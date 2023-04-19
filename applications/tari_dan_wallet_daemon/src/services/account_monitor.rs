//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use log::*;
use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::{
    apis::{
        accounts::AccountsApiError,
        confidential_outputs::ConfidentialOutputsApiError,
        substate::SubstateApiError,
        transaction::TransactionApiError,
    },
    storage::WalletStore,
    DanWalletSdk,
};
use tari_engine_types::{
    substate::{SubstateAddress, SubstateDiff, SubstateValue},
    vault::Vault,
};
use tari_shutdown::ShutdownSignal;
use tari_template_lib::resource::TOKEN_SYMBOL;
use tokio::{time, time::MissedTickBehavior};

use crate::{
    notify::Notify,
    services::{AccountChangedEvent, WalletEvent},
};

const LOG_TARGET: &str = "tari::dan_wallet_daemon::account_monitor";

pub struct AccountMonitor<TStore> {
    notify: Notify<WalletEvent>,
    wallet_sdk: DanWalletSdk<TStore>,
    shutdown_signal: ShutdownSignal,
}

impl<TStore> AccountMonitor<TStore>
where TStore: WalletStore + Clone + Send + Sync + 'static
{
    pub fn new(notify: Notify<WalletEvent>, wallet_sdk: DanWalletSdk<TStore>, shutdown_signal: ShutdownSignal) -> Self {
        Self {
            notify,
            wallet_sdk,
            shutdown_signal,
        }
    }

    pub async fn run(mut self) -> Result<(), anyhow::Error> {
        let mut events_subscription = self.notify.subscribe();
        let mut poll_interval = time::interval(Duration::from_secs(60));
        poll_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = self.shutdown_signal.wait() => {
                    break Ok(());
                }

                _ = poll_interval.tick() => {
                    trace!(target: LOG_TARGET, "Polling for transactions");
                    self.on_poll().await;
                }

                  Ok(event) = events_subscription.recv() => {
                    if let Err(e) = self.on_event(event).await {
                        error!(target: LOG_TARGET, "Error handling event: {}", e);
                    }
                },
            }
        }
    }

    async fn on_poll(&self) {
        if let Err(err) = self.refresh_all_accounts().await {
            error!(target: LOG_TARGET, "Error checking pending transactions: {}", err);
        }
    }

    async fn refresh_all_accounts(&self) -> Result<(), AccountMonitorError> {
        let substate_api = self.wallet_sdk.substate_api();
        let accounts_api = self.wallet_sdk.accounts_api();
        // TODO: There could be more than 100 accounts
        let accounts = accounts_api.get_many(0, 100)?;
        let mut is_updated = false;
        for account in accounts {
            info!(
                target: LOG_TARGET,
                "👁️‍🗨️ Checking balance for account '{}' {}", account.name, account.address
            );
            let account_children = substate_api.load_dependent_substates(&[&account.address])?;
            // TODO: Support detecting new vaults
            let known_child_vaults = account_children
                .iter()
                .filter(|s| s.address.is_vault())
                .collect::<Vec<_>>();
            for vault in known_child_vaults {
                let substate = substate_api.scan_from_vn(&vault.address).await.optional()?;
                let Some((versioned_addr, substate)) = substate else {
                    warn!(target: LOG_TARGET, "Account {} does not exist according to validator node", account.address);
                    continue;
                };
                if versioned_addr.version == vault.version {
                    debug!(target: LOG_TARGET, "Vault {} is up to date", versioned_addr.address);
                    continue;
                }

                let SubstateValue::Vault(vault) = substate else {
                    error!(target: LOG_TARGET, "Substate {} is not a vault. This should be impossible.", vault.address);
                    continue;
                };

                is_updated = true;
                self.refresh_vault(&account.address, &vault)?;
            }

            if is_updated {
                self.notify.notify(AccountChangedEvent {
                    account_address: account.address.clone(),
                });
            } else {
                info!(
                    target: LOG_TARGET,
                    "👁️‍🗨️ Account '{}' {} is up to date", account.name, account.address
                );
            }
        }
        Ok(())
    }

    fn refresh_vault(&self, account_addr: &SubstateAddress, vault: &Vault) -> Result<(), AccountMonitorError> {
        let balance = vault.balance();
        let vault_addr = SubstateAddress::Vault(*vault.vault_id());
        self.wallet_sdk
            .accounts_api()
            .update_vault_balance(&vault_addr, balance)?;
        info!(
            target: LOG_TARGET,
            "👁️‍🗨️ vault {} in account {} has new balance {}",
            vault.vault_id(),
            account_addr,
            balance
        );
        if let Some(outputs) = vault.get_confidential_outputs() {
            info!(
                target: LOG_TARGET,
                "👁️‍🗨️ vault {} in account {} has {} confidential outputs",
                vault.vault_id(),
                account_addr,
                outputs.len()
            );
            self.wallet_sdk
                .confidential_outputs_api()
                .verify_and_update_confidential_outputs(account_addr, &vault_addr, outputs)?;
        }
        Ok(())
    }

    async fn process_result(&self, diff: &SubstateDiff) -> Result<(), AccountMonitorError> {
        let vaults = diff.up_iter().filter(|(a, _)| a.is_vault()).collect::<Vec<_>>();
        for (vault_addr, substate) in vaults {
            let SubstateValue::Vault(vault) = substate.substate_value() else {
                error!(target: LOG_TARGET, "👁️‍🗨️ Substate {} is not a vault. This should be impossible.", vault_addr);
                continue;
            };

            // Try and get the account address from the vault
            let maybe_vault_substate = self.wallet_sdk.substate_api().get_substate(vault_addr).optional()?;
            let Some(vault_substate) = maybe_vault_substate else{
                // This should be impossible.
                error!(target: LOG_TARGET, "👁️‍🗨️ Vault {} is not a known substate.", vault_addr);
                continue;
            };

            let Some(account_addr) = vault_substate.parent_address else {
                warn!(target: LOG_TARGET, "👁️‍🗨️ Vault {} has no parent component. Assuming", vault_addr);
                continue;
            };

            // Check if this vault is associated with an account
            if self
                .wallet_sdk
                .accounts_api()
                .get_account(&account_addr)
                .optional()?
                .is_none()
            {
                info!(
                    target: LOG_TARGET,
                    "👁️‍🗨️ Vault {} not in any known account",
                    vault.vault_id(),
                );
                continue;
            }

            // Add the vault if it does not exist
            if !self.wallet_sdk.accounts_api().has_vault(&vault_addr)? {
                let scan_result = self
                    .wallet_sdk
                    .substate_api()
                    .scan_from_vn(&(*vault.resource_address()).into())
                    .await;
                let maybe_resource = match scan_result {
                    Ok((_, resource)) => {
                        let resx = resource.into_resource().ok_or_else(|| {
                            AccountMonitorError::UnexpectedSubstate(format!(
                                "Expected {} to be a resource.",
                                vault.resource_address()
                            ))
                        })?;
                        Some(resx)
                    },
                    // Dont fail to update if scanning fails
                    Err(err) => {
                        warn!(
                            target: LOG_TARGET,
                            "👁️‍🗨️ Failed to scan vault {} from VN: {}",
                            vault.vault_id(),
                            err
                        );
                        None
                    },
                };

                let token_symbol = maybe_resource.and_then(|r| r.metadata().get(TOKEN_SYMBOL).map(|s| s.to_string()));
                info!(
                    target: LOG_TARGET,
                    "👁️‍🗨️ New {} in account {}",
                    vault.vault_id(),
                    account_addr
                );
                self.wallet_sdk.accounts_api().add_vault(
                    account_addr.clone(),
                    vault_addr.clone(),
                    *vault.resource_address(),
                    vault.resource_type(),
                    token_symbol,
                )?;
            }

            // Update the vault balance / confidential outputs
            self.refresh_vault(&account_addr, vault)?;
        }
        Ok(())
    }

    async fn on_event(&self, event: WalletEvent) -> Result<(), AccountMonitorError> {
        match event {
            WalletEvent::TransactionSubmitted(_) => {},
            WalletEvent::TransactionFinalized(event) => {
                if let Some(diff) = event.finalize.result.accept() {
                    self.process_result(diff).await?;
                }
            },
            WalletEvent::TransactionInvalid(_) => {},
            WalletEvent::AccountChanged(_) => {},
        }
        Ok(())
    }
}

// fn extract_vault_ids(account_state: &[u8]) -> Vec<VaultId> {
//     use std::collections::HashMap;
//
//     use tari_bor::{borsh, decode_exact, Decode};
//     // HACK: This is brittle, the account structure could change
//     #[derive(Decode)]
//     struct AccountDecode {
//         vaults: HashMap<ResourceAddress, tari_template_lib::models::Vault>,
//     }
//
//     let account = decode_exact::<AccountDecode>(account_state).unwrap();
//     account.vaults.values().map(|v| v.vault_id()).collect()
// }

#[derive(Debug, thiserror::Error)]
pub enum AccountMonitorError {
    #[error("Transaction API error: {0}")]
    Transaction(#[from] TransactionApiError),
    #[error("Accounts API error: {0}")]
    Accounts(#[from] AccountsApiError),
    #[error("Substate API error: {0}")]
    Substate(#[from] SubstateApiError),
    #[error("Outputs API error: {0}")]
    ConfidentialOutputs(#[from] ConfidentialOutputsApiError),
    #[error("Unexpected substate: {0}")]
    UnexpectedSubstate(String),
}
