//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A walletd handler context for handler tests, with helpers that mint each
//! kind of bearer a caller can present.

use std::str::FromStr;

use axum_extra::headers::{Authorization, authorization::Bearer};
use tari_ootle_address::Network;
use tari_ootle_wallet_sdk::{
    IndexerTrust,
    WalletSdkConfig,
    cipher_seed::CipherSeedRestore,
    models::EpochBirthday,
    storage::{WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_sdk_services::{
    account_monitor::AccountMonitor,
    indexer_rest_api::IndexerRestApiNetworkInterface,
    notify::Notify,
    transaction_service::TransactionService,
    utxo_scanner::StealthUtxoScannerWorker,
};
use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
use tari_ootle_walletd_client::permissions::Permissions;
use tari_shutdown::Shutdown;
use tari_utilities::SafePassword;

use crate::{
    WalletSdk,
    config::{WalletDaemonAuth, WalletDaemonConfig},
    handlers::{
        HandlerContext,
        auth::{api_keys, cookie, create_authenticator},
    },
};

pub(crate) struct TestDaemon {
    pub context: HandlerContext,
    /// The wallet's own interactive session, holding `admin`.
    pub session: Bearer,
    /// The daemon's local `.cookie` bearer token (`tc_...`).
    pub cookie: Bearer,
    _temp: tempfile::TempDir,
}

impl TestDaemon {
    /// A handler context on LocalNet whose indexer is a closed port, so any
    /// handler path that reaches the network fails fast.
    pub async fn start() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = SqliteWalletStore::try_open(temp.path().join("wallet.sqlite")).unwrap();
        store.run_migrations().unwrap();
        let mut sdk = WalletSdk::initialize_with_local_key_store(
            store.clone(),
            IndexerRestApiNetworkInterface::new("http://127.0.0.1:1"),
            WalletSdkConfig {
                network: Network::LocalNet,
                override_keyring_password: Some(SafePassword::from_str("test wallet password").unwrap()),
                indexer_trust: IndexerTrust::default(),
            },
            EpochBirthday::far_future(),
        )
        .unwrap();
        sdk.initialize_cipher_seed(CipherSeedRestore::CreateNewIfRequired)
            .unwrap();

        let notify = Notify::new(10);
        let shutdown = Shutdown::new();
        let (transaction_service, transaction_service_handle) =
            TransactionService::new(notify.clone(), sdk.clone(), shutdown.to_signal());
        let (utxo_worker, utxo_scanner_handle) = StealthUtxoScannerWorker::new(sdk.clone(), notify.clone()).spawn();
        let (account_monitor, account_monitor_handle) =
            AccountMonitor::new(notify.clone(), sdk.clone(), utxo_scanner_handle, shutdown.to_signal());
        let mut config = WalletDaemonConfig::default();
        config.network = Network::LocalNet;
        config.authentication = WalletDaemonAuth::None;
        let cookie_token = cookie::mint_cookie_token();
        let cookie_permissions = config.parsed_cookie_scopes().unwrap();
        let cookie_str = std::str::from_utf8(cookie_token.reveal()).expect("ASCII cookie token");
        let cookie = Authorization::<Bearer>::bearer(cookie_str).unwrap().0;
        let context = HandlerContext::new(
            sdk,
            notify,
            transaction_service_handle,
            account_monitor_handle,
            config.clone(),
            create_authenticator(&config, store).unwrap(),
            SafePassword::from_str("test jwt secret").unwrap(),
            shutdown.to_signal(),
        )
        .with_cookie_token(Some((cookie_token, cookie_permissions)));

        // Handlers need only the context, so the background workers shut down now.
        shutdown.trigger();
        drop(account_monitor);
        drop(transaction_service);
        utxo_worker.abort();
        drop(utxo_worker.await);

        let session = Self::jwt(&context, "admin", false);
        Self {
            context,
            session,
            cookie,
            _temp: temp,
        }
    }

    /// Stores an API key named `name` holding `permissions`, bypassing the
    /// grant checks of `auth.create_api_key`, and returns its bearer.
    pub fn api_key(&self, name: &str, permissions: &str) -> Bearer {
        let raw = format!("{}{name}", api_keys::API_KEY_PREFIX);
        self.context
            .wallet_sdk()
            .store()
            .with_write_tx(|tx| tx.api_key_insert(name, &api_keys::hash_api_key(&raw), permissions, None))
            .unwrap();
        Authorization::<Bearer>::bearer(&raw).unwrap().0
    }

    /// A token like the one `webrtc.start` mints for a connected app.
    pub fn delegated_session(&self, permissions: &str) -> Bearer {
        Self::jwt(&self.context, permissions, true)
    }

    fn jwt(context: &HandlerContext, permissions: &str, delegated: bool) -> Bearer {
        let mut claims = context
            .jwt_api()
            .generate_auth_claims(Permissions::from_str(permissions).unwrap())
            .unwrap();
        claims.delegated = delegated;
        claims.delegation_id = delegated.then(rand::random);
        Authorization::<Bearer>::bearer(&context.jwt_api().grant(&claims).unwrap())
            .unwrap()
            .0
    }
}
