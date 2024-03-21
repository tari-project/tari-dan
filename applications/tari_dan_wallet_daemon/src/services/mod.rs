//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod events;
pub use events::*;

mod account_monitor;
pub use account_monitor::AccountMonitorHandle;

mod transaction_service;
// -------------------------------- Spawn -------------------------------- //
use anyhow::anyhow;
use futures::{future, future::BoxFuture, FutureExt};
use tari_dan_common_types::optional::IsNotFoundError;
use tari_dan_wallet_sdk::{network::WalletNetworkInterface, storage::WalletStore, DanWalletSdk};
use tari_shutdown::ShutdownSignal;
use tokio::{sync::oneshot, task::JoinHandle};
use transaction_service::TransactionService;
pub use transaction_service::TransactionServiceHandle;

use crate::{notify::Notify, services::account_monitor::AccountMonitor};

type Reply<T> = oneshot::Sender<T>;

pub fn spawn_services<TStore, TNetworkInterface>(
    shutdown_signal: ShutdownSignal,
    notify: Notify<WalletEvent>,
    wallet_sdk: DanWalletSdk<TStore, TNetworkInterface>,
) -> Services
where
    TStore: WalletStore + Clone + Send + Sync + 'static,
    TNetworkInterface: WalletNetworkInterface + Clone + Send + Sync + 'static,
    TNetworkInterface::Error: IsNotFoundError,
{
    let (transaction_service, transaction_service_handle) =
        TransactionService::new(notify.clone(), wallet_sdk.clone(), shutdown_signal.clone());
    let transaction_service_join_handle = tokio::spawn(transaction_service.run());
    let (account_monitor, account_monitor_handle) = AccountMonitor::new(notify, wallet_sdk, shutdown_signal);
    let account_monitor_join_handle = tokio::spawn(account_monitor.run());

    Services {
        account_monitor_handle,
        transaction_service_handle,
        services_fut: try_select_any([transaction_service_join_handle, account_monitor_join_handle]).boxed(),
    }
}

pub struct Services {
    pub services_fut: BoxFuture<'static, Result<(), anyhow::Error>>,
    pub account_monitor_handle: AccountMonitorHandle,
    pub transaction_service_handle: TransactionServiceHandle,
}

async fn try_select_any<I>(handles: I) -> Result<(), anyhow::Error>
where I: IntoIterator<Item = JoinHandle<Result<(), anyhow::Error>>> {
    let (res, _, _) = future::select_all(handles).await;
    match res {
        Ok(res) => res,
        Err(e) => Err(anyhow!("Task panicked: {}", e)),
    }
}
