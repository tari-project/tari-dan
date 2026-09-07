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

use std::{fs, io::IsTerminal, panic, path::PathBuf, process};

use anyhow::Context;
use futures::stream;
use log::*;
use logroller::{Rotation, RotationSize};
use tari_indexer::{cli::Cli, config::ApplicationConfig, run_indexer};
use tari_ootle_app_utilities::configuration::load_configuration;
use tari_shutdown::Shutdown;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{Layer, filter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

const LOG_TARGET: &str = "tari::indexer::app";

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
        eprintln!("CRASH: {:?}", err);
        error!(target: LOG_TARGET, "CRASH: {:?}", err);
        process::exit(1);
    }
}

async fn main_inner() -> anyhow::Result<()> {
    let cli = Cli::init();
    let config_path = cli.common.config_path();
    let cfg = load_configuration(config_path, true, &cli, Some(cli.network()))?;
    let mut config = ApplicationConfig::load_from(&cfg)?;
    // An explicit oracle config file given on the CLI takes precedence over both `config_file` and `config_json`.
    if let Some(path) = cli.epoch_oracle_config.clone() {
        config.epoch_oracle.configured.set_config_file_override(path);
    }
    // Remove the file if it was left behind by a previous run
    let _file = fs::remove_file(config.common.base_path.join("pid"));
    let mut shutdown = Shutdown::new();
    let _guard = init_tracing_subscriber(&cli)?;

    run_indexer(config, stream::empty(), shutdown.to_signal()).await?;
    shutdown.trigger();

    Ok(())
}

fn init_tracing_subscriber(cli: &Cli) -> anyhow::Result<WorkerGuard> {
    let log_dir = cli.common.get_base_path().join("log").join("indexer");
    if !log_dir.exists() {
        fs::create_dir_all(&log_dir).context("Could not create parent directory for log file")?;
    }

    let appender = logroller::LogRollerBuilder::new(&log_dir, &PathBuf::from("indexer.log"))
        .rotation(Rotation::SizeBased(RotationSize::MB(200)))
        .max_keep_files(4)
        .compression(logroller::Compression::Gzip)
        .build()?;
    let (ootle_log, guard) = tracing_appender::non_blocking(appender);

    tracing_subscriber::registry()
        .with(fmt::Layer::new().with_writer(ootle_log).with_ansi(false).with_filter(
            filter::Targets::new().with_targets([
                ("tari::application", tracing::Level::DEBUG),
                ("tari::indexer", tracing::Level::DEBUG),
                ("tari::ootle", tracing::Level::DEBUG),
                ("tower_http", tracing::Level::DEBUG),
            ]),
        ))
        .with(
            fmt::Layer::new()
                .without_time()
                .with_target(false)
                .with_writer(std::io::stdout)
                .with_ansi(std::io::stdout().is_terminal())
                .with_filter(filter::Targets::new().with_targets([
                    ("tari::application", tracing::Level::INFO),
                    ("tari::indexer", tracing::Level::INFO),
                    ("tari::ootle", tracing::Level::INFO),
                ])),
        )
        .try_init()?;

    Ok(guard)
}
