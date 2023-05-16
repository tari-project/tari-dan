//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, time::Duration};

use log::*;
use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_dan_wallet_sdk::{
    apis::{
        accounts::AccountsApiError,
        confidential_outputs::ConfidentialOutputsApiError,
        substate::{SubstateApiError, ValidatorScanResult},
        transaction::TransactionApiError,
    },
    storage::WalletStore,
    substate_provider::WalletNetworkInterface,
    DanWalletSdk,
};
use tari_engine_types::{
    indexed_value::{IndexedValue, ValueVisitorError},
    substate::{Substate, SubstateAddress, SubstateDiff, SubstateValue},
    vault::Vault,
};
use tari_shutdown::ShutdownSignal;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tokio::{
    sync::{mpsc, oneshot},
    time,
    time::MissedTickBehavior,
};

use crate::{
    notify::Notify,
    services::{AccountChangedEvent, Reply, WalletEvent},
};

const LOG_TARGET: &str = "tari::dan::wallet_daemon::account_monitor";

pub struct AccountMonitor<TStore, TNetworkInterface> {
    notify: Notify<WalletEvent>,
    wallet_sdk: DanWalletSdk<TStore, TNetworkInterface>,
    request_rx: mpsc::Receiver<AccountMonitorRequest>,
    shutdown_signal: ShutdownSignal,
}

impl<TStore, TNetworkInterface> AccountMonitor<TStore, TNetworkInterface>
where
    TStore: WalletStore,
    TNetworkInterface: WalletNetworkInterface,
    TNetworkInterface::Error: IsNotFoundError,
{
    pub fn new(
        notify: Notify<WalletEvent>,
        wallet_sdk: DanWalletSdk<TStore, TNetworkInterface>,
        shutdown_signal: ShutdownSignal,
    ) -> (Self, AccountMonitorHandle) {
        let (request_tx, request_rx) = mpsc::channel(1);

        (
            Self {
                notify,
                wallet_sdk,
                request_rx,
                shutdown_signal,
            },
            AccountMonitorHandle { sender: request_tx },
        )
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

                Some(req) = self.request_rx.recv() => {
                    self.handle_request(req).await;
                }

                Ok(event) = events_subscription.recv() => {
                    if let Err(e) = self.on_event(event).await {
                        error!(target: LOG_TARGET, "Error handling event: {}", e);
                    }
                },
            }
        }
    }

    async fn handle_request(&self, req: AccountMonitorRequest) {
        match req {
            AccountMonitorRequest::RefreshAccount { account, reply } => {
                let _ignore = reply.send(self.refresh_account(&account).await);
            },
        }
    }

    async fn on_poll(&self) {
        if let Err(err) = self.refresh_all_accounts().await {
            error!(target: LOG_TARGET, "Error checking pending transactions: {}", err);
        }
    }

    async fn refresh_all_accounts(&self) -> Result<(), AccountMonitorError> {
        let accounts_api = self.wallet_sdk.accounts_api();
        // TODO: There could be more than 100 accounts
        let accounts = accounts_api.get_many(0, 100)?;
        for account in accounts {
            info!(
                target: LOG_TARGET,
                "👁️‍🗨️ Refreshing account '{}' {}", account.name, account.address
            );
            let is_updated = self.refresh_account(&account.address).await?;

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

    async fn refresh_account(&self, account_address: &SubstateAddress) -> Result<bool, AccountMonitorError> {
        let substate_api = self.wallet_sdk.substate_api();
        let accounts_api = self.wallet_sdk.accounts_api();

        let mut is_updated = false;
        let account_substate = substate_api.get_substate(account_address)?;
        let ValidatorScanResult {
            address: versioned_account_address,
            substate: account_value,
            created_by_tx,
        } = substate_api
            .scan_for_substate(
                &account_substate.address.address,
                Some(account_substate.address.version),
            )
            .await?;

        substate_api.save_root(created_by_tx, versioned_account_address.clone())?;

        let vaults_value = IndexedValue::from_raw(&account_value.component().unwrap().state.state)?;
        let known_child_vaults = substate_api
            .load_dependent_substates(&[&account_substate.address.address])?
            .into_iter()
            .filter(|s| s.address.is_vault())
            .map(|s| (s.address, s.version))
            .collect::<HashMap<_, _>>();
        for vault in vaults_value.vault_ids() {
            let vault_addr = SubstateAddress::Vault(*vault);
            let maybe_vault_version = known_child_vaults.get(&vault_addr).copied();
            let scan_result = substate_api
                .scan_for_substate(&vault_addr, maybe_vault_version)
                .await
                .optional()?;
            let Some(ValidatorScanResult { address: versioned_addr, substate, created_by_tx}) = scan_result else {
                warn!(target: LOG_TARGET, "Vault {} for account {} does not exist according to validator node", vault_addr, versioned_account_address);
                continue;
            };

            if let Some(vault_version) = maybe_vault_version {
                // The first time a vault is found, know about the vault substate from the tx result but never added
                // it to the database.
                if versioned_addr.version == vault_version && accounts_api.has_vault(&vault_addr)? {
                    info!(target: LOG_TARGET, "Vault {} is up to date", versioned_addr.address);
                    continue;
                }
            }

            let SubstateValue::Vault(vault) = substate else {
                error!(target: LOG_TARGET, "Substate {} is not a vault. This should be impossible.", vault_addr);
                continue;
            };

            is_updated = true;

            substate_api.save_child(created_by_tx, versioned_account_address.address.clone(), versioned_addr)?;
            self.refresh_vault(&versioned_account_address.address, &vault)?;
        }

        Ok(is_updated)
    }

    fn refresh_vault(&self, account_addr: &SubstateAddress, vault: &Vault) -> Result<(), AccountMonitorError> {
        let accounts_api = self.wallet_sdk.accounts_api();

        let balance = vault.balance();
        let vault_addr = SubstateAddress::Vault(*vault.vault_id());
        if !accounts_api.has_vault(&vault_addr)? {
            info!(
                target: LOG_TARGET,
                "🔒️ NEW vault {} in account {}",
                vault.vault_id(),
                account_addr
            );
            accounts_api.add_vault(
                account_addr.clone(),
                vault_addr.clone(),
                *vault.resource_address(),
                vault.resource_type(),
                // TODO: fetch the token symbol from the resource
                None,
            )?;
        }

        accounts_api.update_vault_balance(&vault_addr, balance)?;
        info!(
            target: LOG_TARGET,
            "🔒️ vault {} in account {} has new balance {}",
            vault.vault_id(),
            account_addr,
            balance
        );
        if let Some(outputs) = vault.get_confidential_outputs() {
            info!(
                target: LOG_TARGET,
                "🔒️ vault {} in account {} has {} confidential outputs",
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
        let accounts_api = self.wallet_sdk.accounts_api();
        let components = diff
            .up_iter()
            .filter(|(a, v)| a.is_component() && is_account(v))
            .map(|(a, _)| a);

        for addr in components {
            if accounts_api.has_account(addr)? {
                self.refresh_account(addr).await?;
            }
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
            WalletEvent::AuthLoginRequest(_) => {},
        }
        Ok(())
    }
}

#[derive(Debug)]
enum AccountMonitorRequest {
    RefreshAccount {
        account: SubstateAddress,
        reply: Reply<Result<bool, AccountMonitorError>>,
    },
}

#[derive(Debug, Clone)]
pub struct AccountMonitorHandle {
    sender: mpsc::Sender<AccountMonitorRequest>,
}

impl AccountMonitorHandle {
    pub async fn refresh_account(&self, account: SubstateAddress) -> Result<bool, AccountMonitorError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(AccountMonitorRequest::RefreshAccount {
                account,
                reply: reply_tx,
            })
            .await
            .map_err(|_| AccountMonitorError::ServiceShutdown)?;
        reply_rx.await.map_err(|_| AccountMonitorError::ServiceShutdown)?
    }
}

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
    #[error("Failed to decode binary value: {0}")]
    DecodeValueFailed(#[from] ValueVisitorError),
    #[error("Monitor service is not running")]
    ServiceShutdown,
}

fn is_account(s: &Substate) -> bool {
    s.substate_value()
        .component()
        .filter(|c| c.template_address == *ACCOUNT_TEMPLATE_ADDRESS)
        .is_some()
}
