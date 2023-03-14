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
mod handlers;
mod jrpc_server;
mod notify;
mod services;

use std::{error::Error, panic, process};

use tari_dan_wallet_sdk::{apis::key_manager, DanWalletSdk, WalletSdkConfig};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;
use tari_shutdown::ShutdownSignal;

use crate::{cli::Cli, handlers::HandlerContext, notify::Notify, services::spawn_services};

const _LOG_TARGET: &str = "tari::dan_wallet_daemon::main";

pub async fn run_tari_dan_wallet_daemon(cli: Cli, shutdown_signal: ShutdownSignal) -> Result<(), Box<dyn Error>> {
    // Uncomment to enable tokio tracing via tokio-console
    // console_subscriber::init();

    // Setup a panic hook which prints the default rust panic message but also exits the process. This makes a panic in
    // any thread "crash" the system instead of silently continuing.
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        default_hook(info);
        process::exit(1);
    }));

    let store = SqliteWalletStore::try_open(cli.base_dir().join("data/wallet.sqlite"))?;
    store.run_migrations()?;

    let params = WalletSdkConfig {
        // TODO: Configure
        password: None,
        validator_node_jrpc_endpoint: cli.validator_node_endpoint(),
    };
    let wallet_sdk = DanWalletSdk::initialize(store, params)?;
    wallet_sdk
        .key_manager_api()
        .get_or_create_initial(key_manager::TRANSACTION_BRANCH)?;
    let notify = Notify::new(100);

    let service_handles = spawn_services(shutdown_signal.clone(), notify.clone(), wallet_sdk.clone());

    let address = cli.listen_address();
    let handlers = HandlerContext::new(wallet_sdk.clone(), notify);
    let listen_fut = jrpc_server::listen(address, handlers, shutdown_signal);

    // Wait for shutdown, or for any service to error
    tokio::select! {
        res = listen_fut => {
            res?;
        },
        res = service_handles => {
            res?;
        },
    }
    Ok(())
}
