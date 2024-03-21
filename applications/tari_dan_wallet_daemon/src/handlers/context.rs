//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_wallet_sdk::DanWalletSdk;
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;

use crate::{
    config::WalletDaemonConfig,
    indexer_jrpc_impl::IndexerJsonRpcNetworkInterface,
    notify::Notify,
    services::{AccountMonitorHandle, TransactionServiceHandle, WalletEvent},
};

#[derive(Debug, Clone)]
pub struct HandlerContext {
    wallet_sdk: DanWalletSdk<SqliteWalletStore, IndexerJsonRpcNetworkInterface>,
    notifier: Notify<WalletEvent>,
    transaction_service: TransactionServiceHandle,
    account_monitor: AccountMonitorHandle,
    config: WalletDaemonConfig,
}

impl HandlerContext {
    pub fn new(
        wallet_sdk: DanWalletSdk<SqliteWalletStore, IndexerJsonRpcNetworkInterface>,
        notifier: Notify<WalletEvent>,
        transaction_service: TransactionServiceHandle,
        account_monitor: AccountMonitorHandle,
        config: WalletDaemonConfig,
    ) -> Self {
        Self {
            wallet_sdk,
            notifier,
            transaction_service,
            account_monitor,
            config,
        }
    }

    pub fn notifier(&self) -> &Notify<WalletEvent> {
        &self.notifier
    }

    pub fn wallet_sdk(&self) -> &DanWalletSdk<SqliteWalletStore, IndexerJsonRpcNetworkInterface> {
        &self.wallet_sdk
    }

    pub fn account_monitor(&self) -> &AccountMonitorHandle {
        &self.account_monitor
    }

    pub fn transaction_service(&self) -> &TransactionServiceHandle {
        &self.transaction_service
    }

    pub fn config(&self) -> &WalletDaemonConfig {
        &self.config
    }
}
