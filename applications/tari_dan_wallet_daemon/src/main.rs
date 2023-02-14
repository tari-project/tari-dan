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

mod cli;
mod handlers;
mod jrpc_server;
mod notify;
mod services;

use std::{error::Error, io::BufWriter, panic, process};

use tari_common::initialize_logging;
use tari_dan_wallet_sdk::{DanWalletSdk, WalletSdkConfig};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;
use tari_shutdown::Shutdown;

use crate::{
    cli::Cli,
    handlers::{HandlerContext, TRANSACTION_KEYMANAGER_BRANCH},
    notify::Notify,
    services::spawn_services,
};

const LOG_TARGET: &str = "tari::dan::wallet_daemon::main";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::init();
    // Uncomment to enable tokio tracing via tokio-console
    // console_subscriber::init();

    // Setup a panic hook which prints the default rust panic message but also exits the process. This makes a panic in
    // any thread "crash" the system instead of silently continuing.
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        default_hook(info);
        process::exit(1);
    }));

    if let Err(e) = initialize_logging(
        cli.base_dir().join("config/logs.yml").as_path(),
        include_str!("../log4rs_sample.yml"),
    ) {
        eprintln!("{}", e);
    }

    let shutdown = Shutdown::new();
    let shutdown_signal = shutdown.to_signal();

    let store = SqliteWalletStore::try_open(cli.base_dir().join("data/wallet.sqlite"))?;
    let mut migration_output = BufWriter::new(Vec::new());
    store.run_migrations_with_output(&mut migration_output)?;
    let migration_output = migration_output.into_inner()?;
    if !migration_output.is_empty() {
        log::info!(target: LOG_TARGET, "{}", String::from_utf8_lossy(&migration_output));
    }

    let params = WalletSdkConfig {
        // TODO: Configure
        password: None,
        validator_node_jrpc_endpoint: cli.validator_node_endpoint(),
    };
    let wallet_sdk = DanWalletSdk::initialize(store, params)?;
    wallet_sdk
        .key_manager_api()
        .get_or_create_initial(TRANSACTION_KEYMANAGER_BRANCH)?;
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
