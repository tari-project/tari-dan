// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use anyhow::{anyhow, bail, Context};
use registration::registration_loop;
use tari_shutdown::{Shutdown, ShutdownSignal};
use tokio::{fs, task::JoinHandle};

use crate::{
    cli::{Cli, Commands},
    config::{get_base_config, Config},
    helpers::read_config_file,
    logger::init_logger,
    manager::{start_receivers, ManagerHandle, ProcessManager},
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

    init_logger()?;

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
            let mut cfg = read_config_file(cli.get_config_path()).await?;
            if let Some(conf) = cfg.missing_conf() {
                bail!("Missing configuration values: {:?}", conf);
            }

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
        _ = async {
            drop(registration_loop(config, manager_handle).await);
        } => {},
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
    let constants = manager_handle.get_consensus_constants(0).await?;
    start_receivers(cr.rx_log, cr.rx_alert, cr.cfg_alert, constants).await;

    Ok(Handlers {
        manager: manager_handle,
        task: cr.task,
    })
}
