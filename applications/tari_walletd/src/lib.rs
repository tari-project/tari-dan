// Copyright 2021. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

pub mod cli;
pub mod config;
mod handlers;
mod server;
mod services;
mod webrtc;

use std::{fs, panic, pin, process};

use log::*;
use tari_common_types::seeds::seed_words::SeedWords;
use tari_crypto::{keys::SecretKey, ristretto::RistrettoSecretKey};
use tari_ootle_address::Network;
use tari_ootle_app_utilities::genesis_resources::{get_public_identity_resource, get_stealth_tari_resource};
use tari_ootle_common_types::{NumPreshards, optional::Optional};
use tari_ootle_wallet_sdk::{
    WalletSdk as Sdk,
    WalletSdkConfig,
    WalletSdkSpec,
    apis::config::{ConfigApi, ConfigKey},
    cipher_seed::CipherSeedRestore,
    local_key_store::LocalKeyStore,
    models::EpochBirthday,
};
use tari_ootle_wallet_sdk_services::{
    account_recovery::AccountRecoveryService,
    indexer_rest_api::IndexerRestApiNetworkInterface,
    notify::Notify,
};
use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
use tari_shutdown::ShutdownSignal;
use tari_utilities::{SafePassword, hex::Hex};
use url::Url;

use crate::{
    config::{ApplicationConfig, WalletDaemonAuth},
    handlers::{
        HandlerContext,
        auth::{cookie::CookieFileGuard, create_authenticator},
    },
    services::spawn_services,
};

const LOG_TARGET: &str = "tari::ootle::wallet_daemon";

// TODO: must match the global network value. All testnets currently have 256 pre-shards.
const NUM_PRESHARDS: NumPreshards = NumPreshards::current();

pub struct OotleWalletDaemonSpec;

impl WalletSdkSpec for OotleWalletDaemonSpec {
    type KeyStore = LocalKeyStore;
    type NetworkInterface = IndexerRestApiNetworkInterface;
    type Store = SqliteWalletStore;
}

pub type WalletSdk = Sdk<OotleWalletDaemonSpec>;

#[allow(clippy::too_many_lines)]
pub async fn run_tari_ootle_walletd(
    config: ApplicationConfig,
    seed_words: Option<&SeedWords>,
    shutdown_signal: ShutdownSignal,
) -> Result<(), anyhow::Error> {
    // Uncomment to enable tokio tracing via tokio-console
    // console_subscriber::init();

    config.ootle_wallet_daemon.validate()?;

    let wallet_store = init_wallet_store(&config)?;
    let mut wallet_sdk: WalletSdk = initialize_wallet_sdk(&config, wallet_store.clone())?;

    info!(
        target: LOG_TARGET,
        "🟢 Starting wallet on {} connected to indexer {} (one of {} configured)",
        wallet_sdk.network(),
        wallet_sdk.get_network_interface().get_endpoint(),
        wallet_sdk.get_network_interface().get_endpoints().len(),
    );

    let needs_seed_recovery =
        wallet_sdk.initialize_cipher_seed(seed_words.map(CipherSeedRestore::FromSeedWords).unwrap_or_default())?;

    // Insert genesis resources
    let (xtr_addr, xtr_resx) = get_stealth_tari_resource(wallet_sdk.network());
    wallet_sdk.resources_api().upsert_resource(&xtr_addr, &xtr_resx)?;
    let (addr, resx) = get_public_identity_resource();
    wallet_sdk.resources_api().upsert_resource(&addr, &resx)?;

    let notify = Notify::new(100);
    let burn_proof_dir = config.ootle_wallet_daemon.get_burn_proof_dir(wallet_sdk.network());
    let services = spawn_services(
        shutdown_signal.clone(),
        notify.clone(),
        wallet_sdk.clone(),
        burn_proof_dir,
        config.ootle_wallet_daemon.auto_claim_burns,
        config.ootle_wallet_daemon.max_fee_limit,
    );

    // trigger account scanning if needed
    if needs_seed_recovery {
        let cipher_seed_birthday = wallet_sdk.key_manager_api().get_cipher_seed_birthday_epoch()?;
        let scanner = AccountRecoveryService::new(
            wallet_sdk.clone(),
            services.account_monitor_handle.clone(),
            services.utxo_scanner_handle.clone(),
            config.ootle_wallet_daemon.recovery_abandon_count,
            cipher_seed_birthday,
        );
        let shutdown_signal = shutdown_signal.clone();
        tokio::spawn(async move {
            let scan_pinned = pin::pin!(scanner.scan());
            shutdown_signal.select(scan_pinned).await;
        });
    }

    let jrpc_address = config.ootle_wallet_daemon.json_rpc_address;
    let signaling_server_address = config.ootle_wallet_daemon.signaling_server_address.unwrap();

    // webauthn

    info!(
        target: LOG_TARGET,
        "🔐 Authentication method set to {:?}",
        config.ootle_wallet_daemon.authentication
    );
    if config.ootle_wallet_daemon.authentication.is_none() {
        warn!(
            target: LOG_TARGET,
            "⚠️ Setting auth to none is dangerous. Your wallet has no password and any program on this computer, \
             or anyone who can reach it over the network, can spend your funds without your consent. If this is \
             not a temporary local-only test wallet, switch authentication=webauthn before sending any value to it.",
        );
    }
    let authenticator = create_authenticator(&config.ootle_wallet_daemon, wallet_store.clone())?;
    if config.ootle_wallet_daemon.enable_webrtc {
        info!(
            target: LOG_TARGET,
            "🔌 WebRTC is enabled: apps can connect through the signaling server at {signaling_server_address}"
        );
    }

    // Generate a new secret each time the daemon starts. This means that all JWT tokens will be invalidated on restart.
    let jwt_secret = create_secret_password();
    let (cookie, _cookie_guard) = if config.ootle_wallet_daemon.cookie_auth {
        let permissions = config.ootle_wallet_daemon.parsed_cookie_scopes()?;
        let (token, guard) = CookieFileGuard::create(&config.common.base_path)?;
        let ceremony_note = match config.ootle_wallet_daemon.authentication {
            WalletDaemonAuth::WebAuthn => " without a WebAuthn ceremony",
            WalletDaemonAuth::None => "",
        };
        warn!(
            target: LOG_TARGET,
            "🍪 Local cookie authentication is enabled ({}, scopes: [{}]). Any process running as this OS user can read \
             the cookie and authenticate{ceremony_note}. Set ootle_wallet_daemon.cookie_auth=false (or pass \
             --cookie-auth=false) to disable it, or narrow ootle_wallet_daemon.cookie_scopes.",
            guard.path().display(),
            config.ootle_wallet_daemon.cookie_scopes.join(", "),
        );
        (Some((token, permissions)), Some(guard))
    } else {
        CookieFileGuard::remove_stale(&config.common.base_path);
        (None, None)
    };
    let handlers = HandlerContext::new(
        wallet_sdk.clone(),
        notify,
        services.transaction_service_handle.clone(),
        services.account_monitor_handle.clone(),
        config.ootle_wallet_daemon.clone(),
        authenticator,
        jwt_secret,
        shutdown_signal,
    )
    .with_cookie_token(cookie);
    let (_jrpc_address, listen_fut) = server::spawn_listener(jrpc_address, signaling_server_address, handlers).await?;

    #[cfg(not(feature = "web_ui"))]
    info!(
        target: LOG_TARGET,
        "Web UI is not enabled. To enable it, add the `web_ui` feature to your Cargo.toml. JSON-RPC address is {jrpc_address}"
    );

    if let Err(e) = fs::write(config.common.base_path.join("pid"), process::id().to_string()) {
        error!(
            target: LOG_TARGET,
            "Failed to create PID file {}: {}",
            config.common.base_path.join("pid").display(),
            e
        )
    }
    // Wait for shutdown, or for any service to error
    tokio::select! {
        res = listen_fut => {
            res??;
        },
        res = services.services_fut => {
            match res {
                Ok(_) => {
                    info!(target: LOG_TARGET, "All services have shut down");
                },
                Err(err) => {
                    error!(target: LOG_TARGET, "🚨 A service has crashed: {}. Shutting down", err);
                    return Err(err);
                },
            }
        },
    }
    Ok(())
}

/// Installs the OS-native credential store that the wallet SDK uses to hold the cipher seed password.
/// Must be called before the SDK touches the keyring. Not needed when `override_keyring_password` is set.
/// If no store can be installed (e.g. no DBus Secret Service on a headless host), a warning is logged and
/// any later keyring access fails with a clear error advising `--override-keyring-password`.
pub fn init_os_keyring_store() {
    let result: keyring_core::Result<std::sync::Arc<keyring_core::CredentialStore>> = {
        #[cfg(target_os = "macos")]
        {
            apple_native_keyring_store::keychain::Store::new().map(|s| s as _)
        }
        #[cfg(target_os = "linux")]
        {
            dbus_secret_service_keyring_store::Store::new().map(|s| s as _)
        }
        #[cfg(target_os = "windows")]
        {
            windows_native_keyring_store::Store::new().map(|s| s as _)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Err(keyring_core::Error::NotSupportedByStore(
                "no OS keyring store is available on this platform".to_string(),
            ))
        }
    };

    match result {
        Ok(store) => keyring_core::set_default_store(store),
        Err(err) => warn!(
            target: LOG_TARGET,
            "⚠️ OS keyring store is unavailable: {err}. The wallet cannot store or retrieve the cipher seed \
             password; use the `--override-keyring-password` option to provide one.",
        ),
    }
}

pub fn init_wallet_store(config: &ApplicationConfig) -> anyhow::Result<SqliteWalletStore> {
    let store = SqliteWalletStore::try_open(config.to_data_dir().join("wallet.sqlite"))?;
    store.run_migrations()?;
    Ok(store)
}

pub fn initialize_wallet_sdk(config: &ApplicationConfig, store: SqliteWalletStore) -> anyhow::Result<WalletSdk> {
    let sdk_config = WalletSdkConfig {
        network: config.ootle_wallet_daemon.network,
        override_keyring_password: config.ootle_wallet_daemon.override_keyring_password.clone(),
        indexer_trust: config.ootle_wallet_daemon.indexer_trust,
    };
    let config_api = ConfigApi::new(&store);
    // Indexer URLs saved through `settings.set` take precedence over the config file.
    let indexer_endpoints = config_api
        .get::<Vec<Url>>(ConfigKey::IndexerUrls)
        .optional()?
        .filter(|urls| !urls.is_empty())
        .unwrap_or_else(|| config.ootle_wallet_daemon.indexer_api_urls.clone());
    let indexer = IndexerRestApiNetworkInterface::init(indexer_endpoints)?;
    let birthday = get_epoch_birthday(sdk_config.network);
    let sdk = WalletSdk::initialize_with_local_key_store(store, indexer, sdk_config, birthday)?;
    Ok(sdk)
}

const fn get_epoch_birthday(network: Network) -> EpochBirthday {
    // TODO: set the zero epoch time for each network according to actual zero epoch time
    match network {
        Network::MainNet => EpochBirthday::far_future(),
        Network::StageNet => EpochBirthday::far_future(),
        Network::NextNet => EpochBirthday::far_future(),
        Network::LocalNet => EpochBirthday::far_future(),
        Network::Igor => EpochBirthday::far_future(),
        Network::Esmeralda => EpochBirthday::far_future(),
    }
}

fn create_secret_password() -> SafePassword {
    let secret = RistrettoSecretKey::random(&mut rand::rng());
    // It is safe to use to_hex() since the String's underlying Vec is moved directly into SafePassword
    SafePassword::from(secret.to_hex())
}
