//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    future::Future,
    pin::{Pin, pin},
};

use anyhow::{Context, anyhow};
use futures::future::{Either, select};
use tari_ootle_wallet_sdk::Network;
use tari_shutdown::Shutdown;
use tokio::fs;

use crate::{
    cli::{Cli, Commands, InitArgs},
    config::{CompileConfig, Config, ExecutableConfig, InstanceConfig, InstanceType, ProcessesConfig, WebserverConfig},
    logger::init_logger,
};

mod cli;
mod config;
mod layer_one_transactions;
mod logfile;
mod logger;
mod process_definitions;
mod process_manager;
mod webserver;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::init();
    let config_path = cli.get_config_path();

    match cli.command {
        Commands::Init(ref args) => {
            init_logger(None)?;
            if config_path.exists() {
                if args.force {
                    log::warn!("Overwriting existing config file at {}", config_path.display());
                } else {
                    log::info!("Config file exists at {}", config_path.display());
                    return Ok(());
                }
            }

            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent).await.context("create_dir_all failed")?;
            }
            let config = get_initial_config(&cli, args)?;
            let file = fs::File::create(&config_path)
                .await
                .with_context(|| anyhow!("Failed to open config path {}", config_path.display()))?;
            config.write(file).await.context("Writing config failed")?;
            let config_path = config_path
                .canonicalize()
                .context("Failed to canonicalize config_path")?;
            log::info!("Config file created at {}", config_path.display());
        },
        Commands::Start(_) => {
            start(&cli).await?;
        },
    }
    Ok(())
}

fn get_initial_config(cli: &Cli, args: &InitArgs) -> anyhow::Result<Config> {
    let mut config = get_base_config(cli)?;
    args.overrides.apply(&mut config)?;
    Ok(config)
}

#[allow(clippy::too_many_lines)]
fn get_base_config(cli: &Cli) -> anyhow::Result<Config> {
    let executables = vec![
        ExecutableConfig {
            instance_type: InstanceType::MinoTariNode,
            // If None, it defaults to the target directory relative to the compile.working_dir for the package
            execuable_path: None,
            compile: Some(CompileConfig {
                working_dir: Some("../tari".into()),
                package_name: "minotari_node".to_string(),
                // Default is "{working_dir}/target/release"
                target_dir: None,
                features: vec![],
                envs: vec![],
            }),
        },
        ExecutableConfig {
            instance_type: InstanceType::MinoTariConsoleWallet,
            execuable_path: None,
            compile: Some(CompileConfig {
                working_dir: Some("../tari".into()),
                package_name: "minotari_console_wallet".to_string(),
                target_dir: None,
                features: vec![],
                envs: vec![],
            }),
        },
        ExecutableConfig {
            instance_type: InstanceType::MinoTariMiner,
            execuable_path: None,
            compile: Some(CompileConfig {
                working_dir: Some("../tari".into()),
                package_name: "minotari_miner".to_string(),
                target_dir: None,
                features: vec![],
                envs: vec![],
            }),
        },
        ExecutableConfig {
            instance_type: InstanceType::TariValidatorNode,
            execuable_path: None,
            compile: Some(CompileConfig {
                working_dir: Some(".".into()),
                package_name: "tari_validator_node".to_string(),
                target_dir: None,
                features: vec!["web_ui".to_string()],
                envs: vec![],
            }),
        },
        ExecutableConfig {
            instance_type: InstanceType::TariIndexer,
            execuable_path: None,
            compile: Some(CompileConfig {
                working_dir: Some(".".into()),
                package_name: "tari_indexer".to_string(),
                target_dir: None,
                features: vec!["web_ui".to_string()],
                envs: vec![],
            }),
        },
        ExecutableConfig {
            instance_type: InstanceType::TariWalletDaemon,
            execuable_path: None,
            compile: Some(CompileConfig {
                working_dir: Some(".".into()),
                package_name: "tari_ootle_walletd".to_string(),
                target_dir: None,
                features: vec!["web_ui".to_string()],
                envs: vec![],
            }),
        },
    ];
    let instances = vec![
        // Generate a key and use that key to assign a claim key for all validators
        InstanceConfig::new(InstanceType::TariWalletDaemonCreateKey)
            .with_name("Wallet Daemon Create Key")
            .use_execution_for(InstanceType::TariWalletDaemon)
            // This relies on the wallet daemon name being "Wallet Daemon" and the wallet daemon base path not being overridden
            .with_base_path_override("wallet-daemon-00")
            .with_num_instances(1),
        InstanceConfig::new(InstanceType::MinoTariNode).with_name("Minotari Node"),
        // WARN: more than one wallet will break things because a random wallet is selected each time (hashmaps) for
        // mining and registrations, so a given wallet is not guaranteed to have funds. There is no big need to fix
        // at the moment this as we typically only need one wallet.
        InstanceConfig::new(InstanceType::MinoTariConsoleWallet)
            .with_name("Minotari Wallet")
            .with_num_instances(1),
        InstanceConfig::new(InstanceType::TariValidatorNode)
            .with_name("Validator node")
            .with_num_instances(1),
        InstanceConfig::new(InstanceType::TariIndexer).with_name("Indexer"),
        // InstanceConfig::new(InstanceType::TariSignalingServer).with_name("Signaling server"),
        InstanceConfig::new(InstanceType::TariWalletDaemon).with_name("Wallet Daemon"),
    ];

    let base_dir = cli
        .common
        .base_dir
        .clone()
        .or_else(|| {
            cli.get_config_path()
                .canonicalize()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap().join("data").join("swarm"));

    std::fs::create_dir_all(&base_dir)?;

    Ok(Config {
        skip_registration: false,
        network: cli.common.network.unwrap_or(Network::LocalNet),
        settings: HashMap::new(),
        start_port: 12000,
        base_dir: base_dir
            .canonicalize()
            .with_context(|| anyhow!("Base path '{}' does not exist", base_dir.display()))?,
        webserver: WebserverConfig::default(),
        processes: ProcessesConfig {
            force_compile: true,
            instances,
            executables,
        },
        enable_manual_validator_connect: false,
        public_ip: None,
        log_to_file: false,
    })
}

async fn start(cli: &Cli) -> anyhow::Result<()> {
    let config_path = cli.get_config_path();
    let mut config = Config::load_with_cli(cli).await.context("failed to load config")?;
    if let Commands::Start(ref overrides) = cli.command {
        overrides.apply(&mut config).context("cli overrides")?;
    }
    let lock_file = config.base_dir.join("tari_swarm.pid");
    let _pid = lockfile::Lockfile::create(&lock_file).with_context(|| {
        anyhow!(
            "Failed to acquire lockfile at '{}'. Is another instance already running? If not, swarm may have \
             previously crashed and you may remove the lockfile.",
            lock_file.display()
        )
    })?;

    create_paths(&config).await?;

    init_logger(config.log_to_file.then_some(config.base_dir.join("logs/")))?;

    log::info!(
        "🐝 Starting swarm on {} (base dir: {}, start port: {})",
        config.network,
        config.base_dir.display(),
        config.start_port
    );

    let mut shutdown = Shutdown::new();
    let signal = shutdown.to_signal().select(exit_signal().context("exit_signal")?);
    let (task_handle, pm_handle) = process_manager::spawn(&config, config_path.clone(), shutdown.to_signal());
    let webserver = webserver::spawn(config, shutdown.to_signal(), pm_handle.clone());

    tokio::select! {
        _ = signal => {
            log::info!("Terminating all instances...");
            terminate_all_instances(&pm_handle).await?;
            shutdown.trigger();
        },
        result = webserver => {
            log::info!("Terminating all instances...");
            terminate_all_instances(&pm_handle).await?;
            result?.context("web server crashed")?;
            log::info!("Webserver exited");
        },
        result = task_handle => {
            // We cannot call terminate_all_instances since the process manager has crashed
            shutdown.trigger();
            result?.context("process manager crashed")?;
            log::info!("Process manager exited");
        }
    }

    Ok(())
}

async fn terminate_all_instances(pm_handle: &process_manager::ProcessManagerHandle) -> anyhow::Result<()> {
    let exit_sig = exit_signal().context("exit_signal")?;
    let stop_all = pm_handle.stop_all();
    match select(exit_sig, pin!(stop_all)).await {
        // If the exit signal was received first (second ctrl+c), we exit immediately
        Either::Left(_) => Err(anyhow!("Received interrupt while stopping instances")),
        Either::Right((r, _)) => {
            let num_instances = r?;
            log::info!("Terminated {num_instances} instances");
            Ok(())
        },
    }
}

async fn create_paths(config: &Config) -> anyhow::Result<()> {
    fs::create_dir_all(&config.base_dir.join("templates"))
        .await
        .context("Failed to create templates directory")?;
    fs::create_dir_all(&config.base_dir.join("misc"))
        .await
        .context("Failed to create misc directory")?;
    fs::create_dir_all(&config.base_dir.join("logs"))
        .await
        .context("Failed to create templates directory")?;
    Ok(())
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

fn exit_signal() -> anyhow::Result<BoxFuture<()>> {
    #[cfg(unix)]
    let fut = unix_exit_signal()?;
    #[cfg(windows)]
    let fut = start_windows()?;

    Ok(fut)
}

#[cfg(unix)]
fn unix_exit_signal() -> anyhow::Result<BoxFuture<()>> {
    use tokio::signal::unix::SignalKind;

    let mut sighup = tokio::signal::unix::signal(SignalKind::hangup())?;
    let mut sigint = tokio::signal::unix::signal(SignalKind::interrupt())?;

    let fut = async move {
        tokio::select! {
            biased;
            _ = sigint.recv() => {
                log::info!("Received SIGINT, shutting down...");
            },
            // This is typically used to signal to reload configuration. Right now we simply exit.
            _ = sighup.recv() => {
                log::info!("Received SIGHUP, shutting down...");
            }
        }
    };

    Ok(Box::pin(fut))
}

#[cfg(windows)]
fn start_windows() -> anyhow::Result<BoxFuture<()>> {
    let mut sigint = tokio::signal::windows::ctrl_c()?;
    let mut sighup = tokio::signal::windows::ctrl_break()?;
    let mut sigshutdown = tokio::signal::windows::ctrl_shutdown()?;
    let fut = async move {
        tokio::select! {
            biased;
            _ = sigint.recv() => {
                log::info!("Received SIGINT, shutting down...");
            },
            _ = sighup.recv() => {
                log::info!("Received SIGHUP, shutting down...");
            }
            _ = sigshutdown.recv() => {
                log::info!("Received SIGSHUTDOWN, shutting down...");
            }
        }
    };
    Ok(Box::pin(fut))
}
