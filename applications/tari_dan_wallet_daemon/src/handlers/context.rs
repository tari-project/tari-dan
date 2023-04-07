//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_wallet_sdk::DanWalletSdk;
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;

use crate::{notify::Notify, services::WalletEvent};

#[derive(Debug, Clone)]
pub struct HandlerContext {
    wallet_sdk: DanWalletSdk<SqliteWalletStore>,
    notifier: Notify<WalletEvent>,
}

impl HandlerContext {
    pub fn new(wallet_sdk: DanWalletSdk<SqliteWalletStore>, notifier: Notify<WalletEvent>) -> Self {
        Self { wallet_sdk, notifier }
    }

    pub fn notifier(&self) -> &Notify<WalletEvent> {
        &self.notifier
    }

    pub fn wallet_sdk(&self) -> &DanWalletSdk<SqliteWalletStore> {
        &self.wallet_sdk
    }
}
