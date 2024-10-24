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

use std::{fs, panic, process};

use clap::Parser;
use log::*;
use tari_common::{
    exit_codes::{ExitCode, ExitError},
    initialize_logging,
};
use tari_dan_app_utilities::configuration::load_configuration;
use tari_shutdown::Shutdown;
use tari_validator_node::{cli::Cli, run_validator_node, ApplicationConfig};

const LOG_TARGET: &str = "tari::validator_node::app";

#[tokio::main]
async fn main() {
    // Uncomment to enable tokio tracing via tokio-console
    // console_subscriber::init();

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
            "Exiting with code ({}) {:?}: {}",
            exit_code as i32,
            exit_code,
            err.details.unwrap_or_default()
        );
        process::exit(exit_code as i32);
    }
}

async fn main_inner() -> Result<(), ExitError> {
    let cli = Cli::parse();
    let config_path = cli.common.config_path();
    let cfg = load_configuration(config_path, true, &cli, cli.common.network)
        .map_err(|e| ExitError::new(ExitCode::ConfigError, e))?;
    let config = ApplicationConfig::load_from(&cfg)?;

    // Remove the pid file if it exists
    let _file = fs::remove_file(config.common.base_path.join("pid"));
    let mut shutdown = Shutdown::new();
    if let Err(e) = initialize_logging(
        &cli.common.log_config_path("validator"),
        &cli.common.get_base_path(),
        include_str!("../log4rs_sample.yml"),
    ) {
        eprintln!("{}", e);
    }

    info!(target: LOG_TARGET, "Starting validator node on network {}", config.network);
    match run_validator_node(&config, shutdown.to_signal()).await {
        Ok(_) => info!(target: LOG_TARGET, "Validator node shutdown successfully"),
        Err(e) => match e.downcast() {
            Ok(exit_error) => {
                error!(target: LOG_TARGET, "Validator node shutdown with an error: {:?}", exit_error);
                return Err(exit_error);
            },
            Err(e) => {
                error!(target: LOG_TARGET, "Validator node shutdown with an error: {:?}", e);
                return Err(ExitError::new(ExitCode::UnknownError, e));
            },
        },
    }

    shutdown.trigger();

    Ok(())
}
