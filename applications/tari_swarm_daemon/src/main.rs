//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{future::Future, pin::Pin};

use anyhow::{anyhow, Context};
use tari_common::configuration::Network;
use tari_shutdown::Shutdown;
use tokio::fs;

use crate::{
    cli::{Cli, Commands, InitArgs},
    config::{CompileConfig, Config, ExecutableConfig, InstanceConfig, InstanceType, ProcessesConfig, WebserverConfig},
};

mod cli;
mod config;
mod process_definitions;
mod process_manager;
mod webserver;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::init();
    let config_path = cli.get_config_path();

    init_logger()?;

    match cli.command {
        Commands::Init(ref args) => {
            if config_path.exists() {
                if args.force {
                    log::warn!("Overwriting existing config file at {}", config_path.display());
                } else {
                    log::info!("Config file exists at {}", config_path.display());
                    return Ok(());
                }
            }

            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent).await?;
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
            // If None, Defaults to the target directory relative to the compile.working_dir for the package
            // We provide this here so that it appears in the config file
            execuable_path: Some("target/release/minotari_node".into()),
            compile: Some(CompileConfig {
                working_dir: Some("../tari".into()),
                package_name: "minotari_node".to_string(),
                // Default is "{working_dir}/target/release"
                target_dir: None,
            }),
            env: vec![],
        },
        ExecutableConfig {
            instance_type: InstanceType::MinoTariConsoleWallet,
            execuable_path: Some("target/release/minotari_console_wallet".into()),
            compile: Some(CompileConfig {
                working_dir: Some("../tari".into()),
                package_name: "minotari_console_wallet".to_string(),
                target_dir: None,
            }),
            env: vec![],
        },
        ExecutableConfig {
            instance_type: InstanceType::MinoTariMiner,
            execuable_path: Some("target/release/minotari_miner".into()),
            compile: Some(CompileConfig {
                working_dir: Some("../tari".into()),
                package_name: "minotari_miner".to_string(),
                target_dir: None,
            }),
            env: vec![],
        },
        ExecutableConfig {
            instance_type: InstanceType::TariValidatorNode,
            execuable_path: Some("target/release/tari_validator_node".into()),
            compile: Some(CompileConfig {
                working_dir: Some(".".into()),
                package_name: "tari_validator_node".to_string(),
                target_dir: None,
            }),
            env: vec![],
        },
        ExecutableConfig {
            instance_type: InstanceType::TariIndexer,
            execuable_path: Some("target/release/tari_indexer".into()),
            compile: Some(CompileConfig {
                working_dir: Some(".".into()),
                package_name: "tari_indexer".to_string(),
                target_dir: None,
            }),
            env: vec![],
        },
        ExecutableConfig {
            instance_type: InstanceType::TariSignalingServer,
            execuable_path: Some("target/release/tari_signaling_server".into()),
            compile: Some(CompileConfig {
                working_dir: Some(".".into()),
                package_name: "tari_signaling_server".to_string(),
                target_dir: None,
            }),
            env: vec![],
        },
        ExecutableConfig {
            instance_type: InstanceType::TariWalletDaemon,
            execuable_path: Some("target/release/tari_wallet_daemon".into()),
            compile: Some(CompileConfig {
                working_dir: Some(".".into()),
                package_name: "tari_dan_wallet_daemon".to_string(),
                target_dir: None,
            }),
            env: vec![],
        },
    ];
    let instances = vec![
        InstanceConfig::new(InstanceType::MinoTariNode).with_name("Minotari Node"),
        // WARN: more than one wallet will break things because a random wallet is selected each time (hashmaps) for
        // mining and registrations, so a given wallet is not guaranteed to have funds. There is no big need to fix
        // at the moment this as we typically only need one wallet.
        InstanceConfig::new(InstanceType::MinoTariConsoleWallet)
            .with_name("Minotari Wallet")
            .with_num_instances(1),
        // Let's mine 10 blocks on startup by default.
        InstanceConfig::new(InstanceType::MinoTariMiner)
            .with_name("Minotari Miner")
            .with_setting("max_blocks", "10"),
        InstanceConfig::new(InstanceType::TariValidatorNode)
            .with_name("Validator node")
            .with_num_instances(1),
        InstanceConfig::new(InstanceType::TariIndexer).with_name("Indexer"),
        InstanceConfig::new(InstanceType::TariSignalingServer).with_name("Signaling server"),
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
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    Ok(Config {
        network: cli.common.network.unwrap_or(Network::LocalNet),
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
    })
}

async fn start(cli: &Cli) -> anyhow::Result<()> {
    let mut config = Config::load_with_cli(cli).await?;
    if let Commands::Start(ref overrides) = cli.command {
        overrides.apply(&mut config)?;
    }
    let _pid = lockfile::Lockfile::create(config.base_dir.join("tari_swarm.pid"))
        .context("Failed to acquire lockfile. Is another instance already running?")?;

    create_paths(&config).await?;

    let shutdown = Shutdown::new();
    let signal = shutdown.to_signal().select(exit_signal()?);
    let (task_handle, pm_handle) = process_manager::spawn(&config, shutdown.to_signal());
    let webserver = webserver::spawn(config, shutdown.to_signal(), pm_handle.clone());

    tokio::select! {
        _ = signal => {
            log::info!("Terminating all instances...");
            let num_instances = pm_handle.stop_all().await?;
            log::info!("Terminated {num_instances} instances");
        },
        result = webserver => {
            log::info!("Terminating all instances...");
            let num_instances = pm_handle.stop_all().await?;
            log::info!("Terminated {num_instances} instances");
            result??;
            log::info!("Webserver exited");
        },
        result = task_handle => {
            result??;
            log::info!("Process manager exited");
        }
    }

    Ok(())
}

async fn create_paths(config: &Config) -> anyhow::Result<()> {
    fs::create_dir_all(&config.base_dir.join("templates"))
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

fn init_logger() -> Result<(), log::SetLoggerError> {
    fn should_skip(target: &str) -> bool {
        const SKIP: [&str; 3] = ["hyper::", "h2::", "tower::"];
        SKIP.iter().any(|s| target.starts_with(s))
    }

    let colors = fern::colors::ColoredLevelConfig::new().info(fern::colors::Color::Green);
    fern::Dispatch::new()
        .format(move |out, message, record| {
            if should_skip(record.target()) {
                return;
            }
            out.finish(format_args!(
                "{} [{}] {} {}",
                humantime::format_rfc3339(std::time::SystemTime::now()),
                record.target(),
                colors.color(record.level()),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .chain(std::io::stdout())
        // .chain(fern::log_file("output.log").unwrap())
        .apply()
}
