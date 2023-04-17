//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod events;
pub use events::*;

mod account_monitor;
mod transaction_service;

// -------------------------------- Spawn -------------------------------- //
use std::future::Future;

use anyhow::anyhow;
use futures::future;
use tari_dan_wallet_sdk::{storage::WalletStore, DanWalletSdk};
use tari_shutdown::ShutdownSignal;
use tokio::task::JoinHandle;
use transaction_service::TransactionService;

use crate::{notify::Notify, services::account_monitor::AccountMonitor};

pub fn spawn_services<TStore>(
    shutdown_signal: ShutdownSignal,
    notify: Notify<WalletEvent>,
    wallet_sdk: DanWalletSdk<TStore>,
) -> impl Future<Output = Result<(), anyhow::Error>>
where
    TStore: WalletStore + Clone + Send + Sync + 'static,
{
    let transaction_service_handle =
        tokio::spawn(TransactionService::new(notify.clone(), wallet_sdk.clone(), shutdown_signal.clone()).run());
    let account_monitor_handle = tokio::spawn(AccountMonitor::new(notify, wallet_sdk, shutdown_signal).run());

    try_select_any([transaction_service_handle, account_monitor_handle])
}

async fn try_select_any<I>(handles: I) -> Result<(), anyhow::Error>
where
    I: IntoIterator<Item = JoinHandle<Result<(), anyhow::Error>>>,
{
    let (res, _, _) = future::select_all(handles).await;
    match res {
        Ok(res) => res,
        Err(e) => Err(anyhow!("Task panicked: {}", e)),
    }
}
