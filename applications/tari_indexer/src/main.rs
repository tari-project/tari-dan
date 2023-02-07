// Copyright 2023. The Tari Project
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
mod json_rpc;

use std::{panic, process};

use cli::Cli;
use log::*;
use tari_common::exit_codes::ExitError;
use tari_engine_types::substate::SubstateAddress;
use tari_shutdown::{Shutdown, ShutdownSignal};
use tokio::{task, time, time::Duration};

use crate::json_rpc::{run_json_rpc, JsonRpcHandlers};

const LOG_TARGET: &str = "tari::indexer::app";
const DEFAULT_POLL_TIME_MS: u64 = 200;

#[tokio::main]
async fn main() {
    // Setup a panic hook which prints the default rust panic message but also exits the process. This makes a panic in
    // any thread "crash" the system instead of silently continuing.
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        default_hook(info);
        process::exit(1);
    }));

    if let Err(err) = main_inner().await {
        let exit_code = err.exit_code;
        eprintln!("{:?}", err);
        error!(
            target: LOG_TARGET,
            "Exiting with code ({}): {:?}", exit_code as i32, exit_code
        );
        process::exit(exit_code as i32);
    }
}

async fn main_inner() -> Result<(), ExitError> {
    let cli = Cli::init();
    let mut shutdown = Shutdown::new();

    run_indexer(cli, shutdown.to_signal()).await?;
    shutdown.trigger();

    Ok(())
}

pub async fn run_indexer(cli: Cli, mut shutdown_signal: ShutdownSignal) -> Result<(), ExitError> {
    // Run the JSON-RPC API
    if let Some(json_rpc_address) = cli.json_rpc_address {
        info!(target: LOG_TARGET, "🌐 Started JSON-RPC server on {}", json_rpc_address);
        let handlers = JsonRpcHandlers::new(cli.address.clone());
        task::spawn(run_json_rpc(json_rpc_address, handlers));
    }

    let poll_time_ms = cli.poll_time_ms.unwrap_or(DEFAULT_POLL_TIME_MS);
    loop {
        tokio::select! {
            _ = time::sleep(Duration::from_millis(poll_time_ms)) => {
                scan_substates(&cli.address).await;
            },

            _ = shutdown_signal.wait() => {
                break;
            },
        }
    }

    Ok(())
}

async fn scan_substates(_addresses: &[SubstateAddress]) {
    // TODO
}
