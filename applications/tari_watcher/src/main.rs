// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::time::SystemTime;

use anyhow::{anyhow, bail, Context};
use helpers::read_registration_file;
use log::*;
use tari_shutdown::{Shutdown, ShutdownSignal};
use tokio::{
    fs, task,
    time::{self, Duration},
};

use crate::{
    cli::{Cli, Commands},
    config::{get_base_config, Config},
    helpers::{contains_key, read_config_file, to_block_height, to_vn_public_keys},
    manager::{ManagerHandle, ProcessManager},
    shutdown::exit_signal,
};

mod cli;
mod config;
mod forker;
mod helpers;
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

async fn start(config: Config) -> anyhow::Result<ManagerHandle> {
    let shutdown = Shutdown::new();
    let signal = shutdown.to_signal().select(exit_signal()?);
    let (task_handle, mut manager_handle) = spawn(config.clone(), shutdown.to_signal()).await;

    let mut interval = time::interval(Duration::from_secs(10));
    let constants = manager_handle.get_consensus_constants(0).await;
    let validity_period = constants.as_ref().unwrap().validator_node_validity_period;
    let epoch_length = constants.unwrap().epoch_length;
    debug!("Registrations are currently valid for {} epochs", validity_period);
    debug!("Every epoch has {} blocks", epoch_length);
    let registration_valid_for = validity_period * epoch_length;
    let mut registered_at_block = 0;
    let local_node = read_registration_file(config.vn_registration_file).await?;
    let local_key = local_node.public_key; // 76fd45c0816f7bd78d33e1b9358a48e8c68b97bfd20d9c80f3934afbde848343
    debug!("Local public key: {}", local_key.clone());

    tokio::select! {
        _ = signal => {
            log::info!("Shutting down");
        },
        result = task_handle => {
            result??;
            log::info!("Process manager exited");
        },
        _ = async {
            loop {
                interval.tick().await;

                let tip_info = manager_handle.get_tip_info().await;
                if let Err(e) = tip_info {
                    error!("Failed to get tip info: {}", e);
                    continue;
                }
                let curr_height = to_block_height(tip_info.unwrap());
                debug!("Current block height: {}", curr_height);

                let vn_status = manager_handle.get_active_validator_nodes().await;
                if let Err(e) = vn_status {
                    error!("Failed to get active validators: {}", e);
                    continue;
                }
                let active_keys = to_vn_public_keys(vn_status.unwrap());
                info!("Amount of active validator node keys: {}", active_keys.len());
                for key in &active_keys {
                    info!("{}", key);
                }

                // if the node is already registered and still valid, skip registration
                if contains_key(active_keys.clone(), local_key.clone()) {
                    info!("Local node is active and still before expiration, skipping registration");
                    continue;
                }

                // need to be more refined but proves the concept
                if curr_height < registered_at_block + registration_valid_for {
                    info!("Local node still within registration validity period, skipping registration");
                    continue;
                }

                info!("Local node not active, attempting to register..");
                let tx = manager_handle.register_validator_node().await.unwrap();
                if !tx.is_success {
                    error!("Failed to register node: {}", tx.failure_message);
                    continue;
                }
                info!("Registered node at height {} with transaction id: {}", curr_height, tx.transaction_id);
                registered_at_block = curr_height;

            }
        } => {},
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
