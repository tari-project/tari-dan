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

use std::{fs, fs::OpenOptions, io::Write, panic, time::SystemTime};

use clap::Parser;
use log::*;
use tari_common::initialize_logging;
use tari_ootle_app_utilities::{configuration::load_configuration, keypair::setup_keypair_prompt};
use tari_shutdown::Shutdown;
use tari_validator_node::{ApplicationConfig, node, run_validator_node};

use crate::cli::Cli;

const LOG_TARGET: &str = "tari::validator_node::app";

#[cfg(feature = "tokio_debug")]
const DEBUG_PORT: u16 = console_subscriber::Server::DEFAULT_PORT;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Setup a panic hook which prints the default rust panic message but also exits the process. This makes a panic in
    // any thread "crash" the system instead of silently continuing.
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        handle_panic(info);
        default_hook(info);
        node::trigger_panic_notifier();
    }));

    let cli = Cli::parse();
    // enable tokio tracing via tokio-console
    #[cfg(feature = "tokio_debug")]
    console_subscriber::Builder::default()
        .server_addr((
            std::net::Ipv4Addr::LOCALHOST,
            cli.tokio_console_port.unwrap_or(DEBUG_PORT),
        ))
        .init();

    let config_path = cli.common.config_path();
    let cfg = load_configuration(config_path, true, &cli, Some(cli.network()))?;
    let mut config = ApplicationConfig::load_from(&cfg)?;
    // An explicit oracle config file given on the CLI takes precedence over both `config_file` and `config_json`.
    if let Some(path) = cli.epoch_oracle_config.clone() {
        config.epoch_oracle.configured.set_config_file_override(path);
    }

    // Remove the pid file if it exists
    let _file = fs::remove_file(config.common.base_path.join("pid")).inspect_err(|e| {
        if e.kind() != std::io::ErrorKind::NotFound {
            warn!(
                target: LOG_TARGET,
                "Failed to remove existing pid file: {}", e
            );
        }
    });
    if let Err(e) = initialize_logging(
        &cli.common.log_config_path("validator"),
        &cli.common.get_base_path(),
        include_str!("../log4rs_sample.yml"),
    ) {
        eprintln!("{}", e);
    }

    match cli.command {
        Some(cli::Subcommand::CompactDb) => {
            let timer = std::time::Instant::now();
            tari_validator_node::consensus::spec::ValidatorNodeStateStore::compact_all(
                &config.validator_node.state_db_path,
            )?;

            info!(
                target: LOG_TARGET,
                "Compacted state database in {:.2?}",
                timer.elapsed()
            );
            return Ok(());
        },
        Some(cli::Subcommand::Start) | None => {
            let shutdown = Shutdown::new();
            let keypair = setup_keypair_prompt(
                &config.validator_node.identity_file,
                !config.validator_node.dont_create_id,
            )?;

            run_validator_node(keypair, config, shutdown).await?;
            info!(target: LOG_TARGET, "Validator node shutdown successfully");
        },
        Some(cli::Subcommand::GenerateIdentity) => {
            let keypair = setup_keypair_prompt(
                &config.validator_node.identity_file,
                !config.validator_node.dont_create_id,
            )?;

            info!(
                target: LOG_TARGET,
                "Generated identity with public key: {}",
                keypair.public_key()
            );
        },
    }

    let metrics = tokio::runtime::Handle::current().metrics();
    info!(
        target: LOG_TARGET,
        "Tokio runtime metrics: num_alive_tasks={}, num_workers={}, global_queue_depth={}",
        metrics.num_alive_tasks(),
        metrics.num_workers(),
        metrics.global_queue_depth(),
    );

    Ok(())
}

fn handle_panic(panic_info: &panic::PanicHookInfo) {
    fn format_current_time() -> String {
        let now = SystemTime::now();
        ::time::OffsetDateTime::from(now)
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|e| format!("format time fail: {e}"))
    }

    let location = panic_info
        .location()
        .map(|loc| format!("file: '{}', line: {}", loc.file(), loc.line()))
        .unwrap_or_else(|| "unknown location".to_string());

    let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
        *s
    } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
        s.as_str()
    } else {
        "Unknown panic message"
    };

    error!(target: LOG_TARGET, "Panic occurred at {location}: {message}");
    tari_validator_node::diagnostics::record_panic(&location, message);

    if let Err(err) = OpenOptions::new()
        .append(true)
        .create(true)
        .open("ootle-node-panic.log")
        .and_then(|mut file| {
            file.write_all(b"---\n")?;
            file.write_all(format!("Timestamp: {}\n", format_current_time()).as_bytes())?;
            file.write_all(format!("Panic at {}: {}\n", location, message).as_bytes())?;
            file.write_all(b"---\n")
        })
    {
        warn!(target: LOG_TARGET, "Failed to write panic log file: {}", err);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn log4rs_sample_config_builds_with_enabled_features() {
        let log_dir = tempfile::tempdir().unwrap();
        let yaml = include_str!("../log4rs_sample.yml").replace("{{log_dir}}", log_dir.path().to_str().unwrap());
        let config: log4rs::config::RawConfig = serde_saphyr::from_str(&yaml).unwrap();
        log4rs::config::create_raw_config(config).unwrap();
    }
}
