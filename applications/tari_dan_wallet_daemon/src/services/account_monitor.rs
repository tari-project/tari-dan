//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, time::Duration};

use log::*;
use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_dan_wallet_sdk::{
    apis::{
        accounts::AccountsApiError,
        confidential_outputs::ConfidentialOutputsApiError,
        non_fungible_tokens::NonFungibleTokensApiError,
        substate::{SubstateApiError, ValidatorScanResult},
        transaction::TransactionApiError,
    },
    models::NonFungibleToken,
    network::WalletNetworkInterface,
    storage::WalletStore,
    DanWalletSdk,
};
use tari_engine_types::{
    indexed_value::{IndexedValue, IndexedValueVisitorError},
    non_fungible::NonFungibleContainer,
    resource::Resource,
    substate::{Substate, SubstateAddress, SubstateDiff, SubstateValue},
    vault::Vault,
};
use tari_shutdown::ShutdownSignal;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    prelude::{NonFungibleId, ResourceAddress},
    resource::TOKEN_SYMBOL,
};
use tari_transaction::TransactionId;
use tokio::{
    sync::{mpsc, oneshot},
    time,
    time::MissedTickBehavior,
};

use crate::{
    notify::Notify,
    services::{AccountChangedEvent, NewAccountInfo, Reply, WalletEvent},
};

const LOG_TARGET: &str = "tari::dan::wallet_daemon::account_monitor";

pub struct AccountMonitor<TStore, TNetworkInterface> {
    notify: Notify<WalletEvent>,
    wallet_sdk: DanWalletSdk<TStore, TNetworkInterface>,
    request_rx: mpsc::Receiver<AccountMonitorRequest>,
    pending_accounts: HashMap<TransactionId, NewAccountInfo>,
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
                pending_accounts: HashMap::new(),
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
            let Some(ValidatorScanResult {
                address: versioned_addr,
                substate,
                created_by_tx,
            }) = scan_result
            else {
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

            self.add_vault_to_account_if_not_exist(&versioned_account_address.address, &vault)
                .await?;
            self.refresh_vault(&versioned_account_address.address, &vault, &HashMap::new())
                .await?;
        }

        Ok(is_updated)
    }

    async fn refresh_vault(
        &self,
        account_addr: &SubstateAddress,
        vault: &Vault,
        nfts: &HashMap<&NonFungibleId, &NonFungibleContainer>,
    ) -> Result<(), AccountMonitorError> {
        let accounts_api = self.wallet_sdk.accounts_api();
        let non_fungibles_api = self.wallet_sdk.non_fungible_api();

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

        if let Some(nft_ids) = vault.get_non_fungible_ids() {
            for id in nft_ids {
                let nft = match nfts.get(id) {
                    Some(nft) => *nft,
                    None => {
                        error!(
                            target: LOG_TARGET,
                            "NonFungible ID {} is found in the vault, but not found in substate diff", id
                        );
                        continue;
                    },
                };

                let is_burned = nft.contents().is_none();
                let nft_contents = if let Some(contents) = nft.contents() {
                    contents
                } else {
                    // TODO: in this case, we are burning an nft, make sure to update the database in that case`
                    continue;
                };
                let metadata = match nft_contents.decode_data() {
                    Ok(data) => data,
                    Err(e) => {
                        error!(
                            target: LOG_TARGET,
                            "Failed to decode non fungible metadata, with error: {}", e
                        );
                        continue;
                    },
                };
                let resource_address = vault.resource_address();
                let resource = self.fetch_resource(*resource_address).await?;
                let token_symbol = resource.token_symbol();

                let non_fungible = NonFungibleToken {
                    is_burned,
                    vault_id: *vault.vault_id(),
                    nft_id: id.clone(),
                    metadata,
                    token_symbol: String::from(token_symbol),
                };

                non_fungibles_api.store_new_nft(&non_fungible)?;
            }
        }
        Ok(())
    }

    async fn process_result(&mut self, tx_id: TransactionId, diff: &SubstateDiff) -> Result<(), AccountMonitorError> {
        let substate_api = self.wallet_sdk.substate_api();
        let accounts_api = self.wallet_sdk.accounts_api();

        if let Some(new_account) = self.pending_accounts.remove(&tx_id) {
            // Filter for a _new_ account created in this transaction
            let new_account_address =
                find_new_account_address(diff).ok_or_else(|| AccountMonitorError::ExpectedNewAccount {
                    tx_id,
                    account_name: new_account.name.clone().unwrap_or_else(|| "<no-name>".to_string()),
                })?;

            accounts_api.add_account(
                new_account.name.as_deref(),
                new_account_address,
                new_account.key_index,
                new_account.is_default,
            )?;
        }

        let mut vaults = diff
            .up_iter()
            .filter(|(a, _)| a.is_vault())
            .map(|(a, s)| (a.as_vault_id().unwrap(), s))
            .collect::<HashMap<_, _>>();

        let accounts = diff
            .up_iter()
            .filter(|(_, s)| is_account(s))
            .filter_map(
                |(a, s)| match IndexedValue::from_raw(&s.substate_value().component().unwrap().state.state) {
                    Ok(value) => Some((a, value)),
                    Err(e) => {
                        error!(
                            target: LOG_TARGET,
                            "👁️‍🗨️ Failed to parse account substate {} in tx {}: {}", a, tx_id, e
                        );
                        None
                    },
                },
            )
            .collect::<Vec<_>>();

        let nfts = diff
            .up_iter()
            .filter_map(|(addr, s)| Some((addr.as_non_fungible_address()?.id(), s.substate_value().non_fungible()?)))
            .collect::<HashMap<_, _>>();

        // Find and process all new vaults
        for (account_addr, value) in accounts {
            for vault_id in value.vault_ids() {
                // Any vaults we process here do not need to be reprocesed later
                if let Some(vault) = vaults.remove(vault_id).and_then(|s| s.substate_value().vault()) {
                    self.add_vault_to_account_if_not_exist(account_addr, vault).await?;
                    self.refresh_vault(account_addr, vault, &nfts).await?;
                }
            }
        }

        // Process all existing vaults that belong to an account
        for (vault_addr, substate) in vaults {
            let vault_addr = SubstateAddress::Vault(vault_addr);
            let SubstateValue::Vault(vault) = substate.substate_value() else {
                error!(target: LOG_TARGET, "👁️‍🗨️ Substate {} is not a vault. This should be impossible.", vault_addr);
                continue;
            };

            // Try and get the account address from the vault
            let maybe_vault_substate = substate_api.get_substate(&vault_addr).optional()?;
            let Some(vault_substate) = maybe_vault_substate else {
                // This should be impossible.
                error!(target: LOG_TARGET, "👁️‍🗨️ Vault {} is not a known substate.", vault_addr);
                continue;
            };

            let Some(account_addr) = vault_substate.parent_address else {
                warn!(target: LOG_TARGET, "👁️‍🗨️ Vault {} has no parent component. Assuming", vault_addr);
                continue;
            };

            // Check if this vault is associated with an account
            if accounts_api.get_account_by_address(&account_addr).optional()?.is_none() {
                info!(
                    target: LOG_TARGET,
                    "👁️‍🗨️ Vault {} not in any known account",
                    vault.vault_id(),
                );
                continue;
            }

            self.add_vault_to_account_if_not_exist(&account_addr, vault).await?;

            // Update the vault balance / confidential outputs
            self.refresh_vault(&account_addr, vault, &nfts).await?;
        }
        Ok(())
    }

    async fn fetch_resource(&self, resx_addr: ResourceAddress) -> Result<Resource, AccountMonitorError> {
        let substate_api = self.wallet_sdk.substate_api();
        let resx_addr = SubstateAddress::Resource(resx_addr);
        let version = substate_api
            .get_substate(&resx_addr)
            .optional()?
            .map(|s| s.address.version)
            .unwrap_or(0);
        let ValidatorScanResult { substate: resource, .. } =
            substate_api.scan_for_substate(&resx_addr, Some(version)).await?;
        let resx = resource.into_resource().ok_or_else(|| {
            AccountMonitorError::UnexpectedSubstate(format!("Expected {} to be a resource.", resx_addr))
        })?;
        Ok(resx)
    }

    async fn add_vault_to_account_if_not_exist(
        &self,
        account_addr: &SubstateAddress,
        vault: &Vault,
    ) -> Result<(), AccountMonitorError> {
        let vault_addr = SubstateAddress::Vault(*vault.vault_id());
        let accounts_api = self.wallet_sdk.accounts_api();
        if accounts_api.has_vault(&vault_addr)? {
            return Ok(());
        }
        let maybe_resource = match self.fetch_resource(*vault.resource_address()).await {
            Ok(r) => Some(r),
            Err(e) => {
                warn!(
                    target: LOG_TARGET,
                    "👁️‍🗨️ Failed to scan vault {} from VN: {}",
                    vault.vault_id(),
                    e
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
        accounts_api.add_vault(
            account_addr.clone(),
            vault_addr,
            *vault.resource_address(),
            vault.resource_type(),
            token_symbol,
        )?;

        Ok(())
    }

    async fn on_event(&mut self, event: WalletEvent) -> Result<(), AccountMonitorError> {
        match event {
            WalletEvent::TransactionSubmitted(event) => {
                if let Some(account) = event.new_account {
                    self.pending_accounts.insert(event.transaction_id, account);
                }
            },
            WalletEvent::TransactionFinalized(event) => {
                if let Some(diff) = event.finalize.result.accept() {
                    self.process_result(event.transaction_id, diff).await?;
                }
            },
            WalletEvent::TransactionInvalid(event) => {
                self.pending_accounts.remove(&event.transaction_id);
            },
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
    #[error("Non Fungibles API error: {0}")]
    NonFungibleTokens(#[from] NonFungibleTokensApiError),
    #[error("Failed to decode binary value: {0}")]
    DecodeValueFailed(#[from] IndexedValueVisitorError),
    #[error("Unexpected substate: {0}")]
    UnexpectedSubstate(String),
    #[error("Monitor service is not running")]
    ServiceShutdown,

    #[error("Expected new account '{account_name}'to be created in transaction {tx_id}")]
    ExpectedNewAccount { tx_id: TransactionId, account_name: String },
}

fn find_new_account_address(diff: &SubstateDiff) -> Option<&SubstateAddress> {
    // TODO: We assume only one new account is created in a transaction.
    diff.up_iter().find_map(|(a, v)| {
        // Newly created in this transaction
        if v.version() > 0 {
            return None;
        }

        // Is an account component
        if !a.is_component() ||
            v.substate_value()
                .component()
                .expect("Value was not component for component address")
                .template_address !=
                *ACCOUNT_TEMPLATE_ADDRESS
        {
            return None;
        }

        Some(a)
    })
}

fn is_account(s: &Substate) -> bool {
    s.substate_value()
        .component()
        .filter(|c| c.template_address == *ACCOUNT_TEMPLATE_ADDRESS)
        .is_some()
}
