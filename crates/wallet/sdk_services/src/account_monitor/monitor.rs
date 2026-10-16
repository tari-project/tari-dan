//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, time::Duration};

use log::*;
use tari_engine_types::{indexed_value::IndexedValueError, resource::Resource};
use tari_ootle_common_types::optional::{IsNotFoundError, Optional};
use tari_ootle_transaction::TransactionId;
use tari_ootle_wallet_sdk::{
    WalletSdk,
    WalletSdkSpec,
    apis::{
        accounts::AccountsApiError,
        confidential_outputs::ConfidentialOutputsApiError,
        non_fungible_tokens::NonFungibleTokensApiError,
        resources::ResourcesApiError,
        stealth_outputs::StealthOutputsApiError,
        substate::SubstateApiError,
        transaction::TransactionApiError,
    },
    models::{BalanceChangeSource, NewAccountData, WalletEvent},
};
use tari_shutdown::ShutdownSignal;
use tari_template_lib_types::{ComponentAddress, ResourceAddress};
use tokio::{
    sync::{broadcast, mpsc},
    time,
    time::MissedTickBehavior,
};

use crate::{
    account_monitor::{
        handle::{AccountMonitorHandle, AccountMonitorRequest},
        scanner::AccountScanner,
    },
    notify::Notify,
    utxo_scanner::UtxoScannerHandle,
};

const LOG_TARGET: &str = "tari::ootle::wallet_services::account_monitor";

pub struct AccountMonitor<TSpec: WalletSdkSpec> {
    notify_subscription: broadcast::Receiver<WalletEvent>,
    wallet_sdk: WalletSdk<TSpec>,
    request_rx: mpsc::Receiver<AccountMonitorRequest>,
    pending_accounts: HashMap<TransactionId, NewAccountData>,
    utxo_scanner_handle: UtxoScannerHandle,
    periodic_scan_interval: Duration,
    enable_periodic_scanning_of_utxos: bool,
    scanner: AccountScanner<TSpec>,
    shutdown_signal: ShutdownSignal,
}

impl<TSpec> AccountMonitor<TSpec>
where
    TSpec: WalletSdkSpec,
    WalletSdk<TSpec>: Clone,
{
    pub fn new(
        notify: Notify<WalletEvent>,
        wallet_sdk: WalletSdk<TSpec>,
        utxo_scanner_handle: UtxoScannerHandle,
        shutdown_signal: ShutdownSignal,
    ) -> (Self, AccountMonitorHandle) {
        let (request_tx, request_rx) = mpsc::channel(1);

        (
            Self {
                notify_subscription: notify.subscribe(),
                wallet_sdk: wallet_sdk.clone(),
                request_rx,
                pending_accounts: HashMap::new(),
                periodic_scan_interval: Duration::from_secs(60),
                utxo_scanner_handle,
                enable_periodic_scanning_of_utxos: true,
                scanner: AccountScanner::new(notify, wallet_sdk),
                shutdown_signal,
            },
            AccountMonitorHandle { sender: request_tx },
        )
    }

    pub fn with_periodic_scan_interval(mut self, interval: Duration) -> Self {
        self.periodic_scan_interval = interval;
        self
    }

    pub fn disable_periodic_scanning_of_utxos(mut self) -> Self {
        self.enable_periodic_scanning_of_utxos = false;
        self
    }

    pub async fn run(mut self) -> Result<(), anyhow::Error> {
        info!(target: LOG_TARGET, "🏦 Account monitor started");
        let mut poll_interval = time::interval(self.periodic_scan_interval);
        poll_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = self.shutdown_signal.wait() => {
                    info!(target: LOG_TARGET, "🏦 Account monitor shutting down");
                    break Ok(());
                }

                _ = poll_interval.tick() => {
                    trace!(target: LOG_TARGET, "Polling for transactions");
                    self.on_poll().await;
                }

                Some(req) = self.request_rx.recv() => {
                    self.handle_request(req).await;
                }

                Ok(event) = self.notify_subscription.recv() => {
                    if let Err(e) = self.on_event(event).await {
                        error!(target: LOG_TARGET, "Error handling event: {}", e);
                    }
                },
            }
        }
    }

    async fn handle_request(&self, req: AccountMonitorRequest) {
        debug!(target: LOG_TARGET, "🏦 Account monitor received request: {:?}", req);
        match req {
            AccountMonitorRequest::RefreshAccount {
                account,
                scan_for_utxos,
                source,
                reply,
            } => {
                let _ignore = reply.send(self.refresh_account(account, scan_for_utxos, source).await);
            },
            AccountMonitorRequest::AssociateResource {
                account,
                resource,
                reply,
            } => {
                let _ignore = reply.send(self.associate_resource_with_account(&account, resource).await);
            },
        }
    }

    async fn associate_resource_with_account(
        &self,
        account_address: &ComponentAddress,
        resource_address: ResourceAddress,
    ) -> Result<(), AccountMonitorError> {
        let accounts_api = self.wallet_sdk.accounts_api();
        self.fetch_and_cache_resource(&resource_address).await?;
        accounts_api.associate_stealth_resource(account_address, resource_address)?;
        Ok(())
    }

    async fn fetch_and_cache_resource(&self, resx_addr: &ResourceAddress) -> Result<Resource, AccountMonitorError> {
        if let Some(resx) = self.wallet_sdk.resources_api().get(resx_addr).optional()? {
            return Ok(resx);
        }

        let resource = self.wallet_sdk.substate_api().fetch_resource(*resx_addr).await?;
        self.wallet_sdk.resources_api().upsert_resource(resx_addr, &resource)?;
        Ok(resource)
    }

    async fn on_poll(&self) {
        if let Err(err) = self.refresh_all_accounts().await {
            error!(target: LOG_TARGET, "Error refreshing all accounts: {}", err);
        }
    }

    async fn refresh_all_accounts(&self) -> Result<(), AccountMonitorError> {
        let accounts_api = self.wallet_sdk.accounts_api();
        const PAGE_SIZE: usize = 10;
        let mut offset = 0;
        loop {
            let accounts = accounts_api.get_many(offset, PAGE_SIZE)?;
            if accounts.is_empty() {
                break;
            }
            for account in &accounts {
                let is_updated = self.scanner.refresh_account(*account.component_address()).await?;
                if self.enable_periodic_scanning_of_utxos {
                    self.refresh_stealth_utxos(*account.component_address()).await?;
                }

                if is_updated {
                    info!(
                        target: LOG_TARGET,
                        "🏦 Account {} has been updated", account
                    );
                } else {
                    info!(
                        target: LOG_TARGET,
                        "🏦 Account {} is up to date", account
                    );
                }
            }
            offset += accounts.len();
            if accounts.len() < PAGE_SIZE {
                break;
            }
        }

        Ok(())
    }

    async fn refresh_account(
        &self,
        account_address: ComponentAddress,
        scan_for_utxos: bool,
        source: BalanceChangeSource,
    ) -> Result<bool, AccountMonitorError> {
        let is_updated = self
            .scanner
            .refresh_account_with_source(account_address, source)
            .await?;
        if scan_for_utxos {
            self.refresh_stealth_utxos(account_address).await?;
        }
        if is_updated {
            info!(
                target: LOG_TARGET,
                "🏦 Account {} updated", account_address
            );
        } else {
            info!(
                target: LOG_TARGET,
                "🏦 Account {} is up to date", account_address
            );
        }
        Ok(is_updated)
    }

    async fn refresh_stealth_utxos(&self, account_address: ComponentAddress) -> Result<(), AccountMonitorError> {
        let associated_resources = self
            .wallet_sdk
            .accounts_api()
            .get_associated_stealth_resources(&account_address)?;

        info!(
            target: LOG_TARGET,
            "🏦 Requesting UTXO scan for account {} for {} stealth resource(s)",
            account_address,
            associated_resources.len()
        );
        for resource_address in associated_resources {
            self.utxo_scanner_handle.request_scan(account_address, resource_address);
        }
        Ok(())
    }

    async fn on_event(&mut self, event: WalletEvent) -> Result<(), AccountMonitorError> {
        debug!(target: LOG_TARGET, "🏦 Account monitor received event: {}", event);
        if let Err(err) = self.wallet_sdk.event_api().log_event(&event) {
            warn!(target: LOG_TARGET, "Failed to store wallet event: {}", err);
        }
        match event {
            WalletEvent::TransactionSubmitted(event) => {
                if let Some(account) = event.context.and_then(|c| c.new_account_data().cloned()) {
                    self.pending_accounts.insert(event.transaction_id, account);
                }
            },
            WalletEvent::TransactionFinalized(event) => {
                if let Some(diff) = event.finalize.result.any_accept() {
                    let new_account = self.pending_accounts.remove(&event.transaction_id);
                    self.scanner
                        .process_result(event.transaction_id, diff, new_account)
                        .await?;
                }
            },
            WalletEvent::TransactionInvalid(event) => {
                self.pending_accounts.remove(&event.transaction_id);
            },
            WalletEvent::UtxoRecovered(event) => {
                self.scanner
                    .record_stealth_balance_change(event.account_address, *event.address.resource_address())?;
            },
            WalletEvent::UtxoSpent(event) => {
                self.scanner
                    .record_stealth_balance_change(event.account_address, *event.address.resource_address())?;
            },
            WalletEvent::AccountCreatedOnChain(_) |
            WalletEvent::AccountChangedOnChain(_) |
            WalletEvent::AuthLoginRequest(_) |
            WalletEvent::UtxoRecoveryStarted(_) |
            WalletEvent::UtxoRecoveryCompleted(_) |
            WalletEvent::TransactionRequestCreated(_) |
            WalletEvent::ValidatorSetReanchored(_) => {},
        }
        Ok(())
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
    #[error("Stealth Outputs API error: {0}")]
    StealthOutputs(#[from] StealthOutputsApiError),
    #[error("Non Fungibles API error: {0}")]
    NonFungibleTokens(#[from] NonFungibleTokensApiError),
    #[error("Resources API error: {0}")]
    Resources(#[from] ResourcesApiError),
    #[error("Failed to decode binary value: {0}")]
    DecodeValueFailed(#[from] IndexedValueError),
    #[error("Unexpected substate: {0}")]
    UnexpectedSubstate(String),
    #[error("Monitor service is not running")]
    ServiceShutdown,
}

impl IsNotFoundError for AccountMonitorError {
    fn is_not_found_error(&self) -> bool {
        match self {
            Self::Transaction(e) => e.is_not_found_error(),
            Self::Accounts(e) => e.is_not_found_error(),
            Self::Substate(e) => e.is_not_found_error(),
            Self::ConfidentialOutputs(e) => e.is_not_found_error(),
            Self::StealthOutputs(e) => e.is_not_found_error(),
            Self::NonFungibleTokens(e) => e.is_not_found_error(),
            Self::Resources(e) => e.is_not_found_error(),
            Self::DecodeValueFailed(_) | Self::UnexpectedSubstate(_) | Self::ServiceShutdown => false,
        }
    }
}
