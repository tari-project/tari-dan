//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_wallet_sdk::DanWalletSdk;
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;

use crate::{jwt::Jwt, notify::Notify, services::WalletEvent};

#[derive(Debug, Clone)]
pub struct HandlerContext {
    wallet_sdk: DanWalletSdk<SqliteWalletStore>,
    notifier: Notify<WalletEvent>,
    jwt: Jwt,
}

impl HandlerContext {
    pub fn new(wallet_sdk: DanWalletSdk<SqliteWalletStore>, notifier: Notify<WalletEvent>, jwt: Jwt) -> Self {
        Self {
            wallet_sdk,
            notifier,
            jwt,
        }
    }

    pub fn notifier(&self) -> &Notify<WalletEvent> {
        &self.notifier
    }

    pub fn wallet_sdk(&self) -> &DanWalletSdk<SqliteWalletStore> {
        &self.wallet_sdk
    }

    pub fn jwt(&self) -> &Jwt {
        &self.jwt
    }
}
