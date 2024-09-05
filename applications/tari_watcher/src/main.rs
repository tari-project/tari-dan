// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use anyhow::{anyhow, Context};
use registration::registration_loop;
use tari_shutdown::{Shutdown, ShutdownSignal};
use tokio::{fs, task::JoinHandle};

use crate::{
    cli::{Cli, Commands},
    config::{get_base_config, Config},
    constants::DEFAULT_WATCHER_BASE_PATH,
    helpers::read_config_file,
    logger::init_logger,
    manager::{start_receivers, ManagerHandle, ProcessManager},
    process::create_pid_file,
    shutdown::exit_signal,
};

mod alerting;
mod cli;
mod config;
mod constants;
mod helpers;
mod logger;
mod manager;
mod minotari;
mod monitoring;
mod process;
mod registration;
mod shutdown;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::init();
    let config_path = cli.get_config_path();

    let config_path = config_path
        .canonicalize()
        .context("Failed to canonicalize config path")?;

    init_logger()?;

    match cli.command {
        Commands::Init(ref args) => {
            // set by default in CommonCli
            let parent = config_path.parent().context("parent path")?;
            fs::create_dir_all(parent).await?;

            let mut config = get_base_config(&cli)?;
            // optionally disables auto register
            args.apply(&mut config);

            let file = fs::File::create(&config_path)
                .await
                .with_context(|| anyhow!("Failed to open config path {}", config_path.display()))?;
            config.write(file).await.context("Writing config failed")?;

            log::info!("Config file created at {}", config_path.display());
        },
        Commands::Start(ref args) => {
            log::info!("Starting watcher using config {}", config_path.display());
            let mut cfg = read_config_file(config_path).await.context("read config file")?;

            // optionally override config values
            args.apply(&mut cfg);
            start(cfg).await?;
        },
    }

    Ok(())
}

async fn start(config: Config) -> anyhow::Result<()> {
    let shutdown = Shutdown::new();
    let signal = shutdown.to_signal().select(exit_signal()?);
    fs::create_dir_all(config.base_dir.join(DEFAULT_WATCHER_BASE_PATH))
        .await
        .context("create watcher base path")?;
    create_pid_file(
        config.base_dir.join(DEFAULT_WATCHER_BASE_PATH).join("watcher.pid"),
        std::process::id(),
    )
    .await?;
    let handlers = spawn_manager(config.clone(), shutdown.to_signal(), shutdown).await?;
    let manager_handle = handlers.manager;
    let task_handle = handlers.task;

    tokio::select! {
        _ = signal => {
            log::info!("Shutting down");
        },
        result = task_handle => {
            result?;
            log::info!("Process manager exited");
        },
        Err(err) = registration_loop(config, manager_handle) => {
            log::error!("Registration loop exited with error {err}");
        },
    }

    Ok(())
}

struct Handlers {
    manager: ManagerHandle,
    task: JoinHandle<()>,
}

async fn spawn_manager(config: Config, shutdown: ShutdownSignal, trigger: Shutdown) -> anyhow::Result<Handlers> {
    let (manager, mut manager_handle) = ProcessManager::new(config, shutdown, trigger);
    let cr = manager.start_request_handler().await?;
    let status = manager_handle.get_tip_info().await?;
    // in the case the consensus constants have changed since the genesis block, use the latest ones
    let constants = manager_handle.get_consensus_constants(status.height()).await?;
    start_receivers(cr.rx_log, cr.rx_alert, cr.cfg_alert, constants).await;

    Ok(Handlers {
        manager: manager_handle,
        task: cr.task,
    })
}
