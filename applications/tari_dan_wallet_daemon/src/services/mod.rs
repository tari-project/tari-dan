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
use tari_dan_wallet_sdk::{storage::WalletStore, DanWalletSdk};
use tari_shutdown::ShutdownSignal;
use tokio::{sync::oneshot, task::JoinHandle};
use transaction_service::TransactionService;

use crate::{notify::Notify, services::account_monitor::AccountMonitor};

pub(self) type Reply<T> = oneshot::Sender<T>;

pub fn spawn_services<TStore>(
    shutdown_signal: ShutdownSignal,
    notify: Notify<WalletEvent>,
    wallet_sdk: DanWalletSdk<TStore>,
) -> Services
where
    TStore: WalletStore + Clone + Send + Sync + 'static,
{
    let transaction_service_join_handle =
        tokio::spawn(TransactionService::new(notify.clone(), wallet_sdk.clone(), shutdown_signal.clone()).run());
    let (account_monitor, account_monitor_handle) = AccountMonitor::new(notify, wallet_sdk, shutdown_signal);
    let account_monitor_join_handle = tokio::spawn(account_monitor.run());

    Services {
        account_monitor_handle,
        services_fut: try_select_any([transaction_service_join_handle, account_monitor_join_handle]).boxed(),
    }
}

pub struct Services {
    pub services_fut: BoxFuture<'static, Result<(), anyhow::Error>>,
    pub account_monitor_handle: AccountMonitorHandle,
}

async fn try_select_any<I>(handles: I) -> Result<(), anyhow::Error>
where I: IntoIterator<Item = JoinHandle<Result<(), anyhow::Error>>> {
    let (res, _, _) = future::select_all(handles).await;
    match res {
        Ok(res) => res,
        Err(e) => Err(anyhow!("Task panicked: {}", e)),
    }
}
