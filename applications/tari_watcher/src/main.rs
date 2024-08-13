// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use crate::manager::ManagerHandle;
use crate::shutdown::exit_signal;
use anyhow::bail;
use log::*;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tari_shutdown::ShutdownSignal;
use tokio::task;

use crate::{
    cli::{Cli, Commands},
    config::{get_base_config, Config},
    manager::ProcessManager,
};
use anyhow::{anyhow, Context};
use tari_shutdown::Shutdown;
use tokio::fs;

mod cli;
mod config;
mod forker;
mod manager;
mod minotari;
mod shutdown;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::init();
    let config_path = cli.get_config_path();

    setup_logger()?;

    match cli.command {
        Commands::Init(ref args) => {
            // set by default in CommonCli
            let parent = config_path.parent().unwrap();
            fs::create_dir_all(parent).await?;

            let mut config = get_base_config(&cli)?;
            // optionally disables auto register
            args.apply(&mut config);

            let file = fs::File::create(&config_path)
                .await
                .with_context(|| anyhow!("Failed to open config path {}", config_path.display()))?;
            config.write(file).await.context("Writing config failed")?;

            let config_path = config_path
                .canonicalize()
                .context("Failed to canonicalize config path")?;

            log::info!("Config file created at {}", config_path.display());
        },
        Commands::Start(ref args) => {
            let mut cfg = read_file(cli.get_config_path()).await?;
            if let Some(conf) = cfg.missing_conf() {
                bail!("Missing configuration values: {:?}", conf);
            }

            // optionally override config values
            args.apply(&mut cfg);
            let _ = start(cfg).await?;
        },
    }

    Ok(())
}

async fn read_file(path: PathBuf) -> anyhow::Result<Config> {
    let p = Path::new(path.to_str().unwrap());
    let content: String = fs::read_to_string(p).await.unwrap();
    let config: Config = toml::from_str(&content)?;

    Ok(config)
}

async fn start(config: Config) -> anyhow::Result<ManagerHandle> {
    let shutdown = Shutdown::new();
    let signal = shutdown.to_signal().select(exit_signal()?);
    let (task_handle, mut manager_handle) = spawn(config.clone(), shutdown.to_signal()).await;

    // Test ping #1 to base node
    let tip = manager_handle.get_tip_info().await;
    info!("[TEST] Tip status: {:?}", tip);

    // Test ping #2 to base node
    let vn_status = manager_handle.get_active_validator_nodes().await;
    info!("[TEST] Active validators: {:?}", vn_status);

    tokio::select! {
        _ = signal => {
            log::info!("Shutting down");
        },
        result = task_handle => {
            result??;
            log::info!("Process manager exited");
        }
    }

    Ok(manager_handle)
}

async fn spawn(config: Config, shutdown: ShutdownSignal) -> (task::JoinHandle<anyhow::Result<()>>, ManagerHandle) {
    let (manager, manager_handle) = ProcessManager::new(config, shutdown);
    let task_handle = tokio::spawn(manager.start());
    (task_handle, manager_handle)
}

fn setup_logger() -> Result<(), fern::InitError> {
    let colors = fern::colors::ColoredLevelConfig::new()
        .info(fern::colors::Color::Green)
        .debug(fern::colors::Color::Cyan)
        .warn(fern::colors::Color::Yellow)
        .error(fern::colors::Color::Red);

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "[{} {} {}] {}",
                humantime::format_rfc3339_seconds(SystemTime::now()),
                colors.color(record.level()),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .chain(std::io::stdout())
        .chain(fern::log_file("output.log")?)
        .apply()?;

    Ok(())
}
