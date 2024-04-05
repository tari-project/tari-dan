//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fs, future::Future, pin::Pin};

use anyhow::Context;
use tari_shutdown::Shutdown;
use tokio::signal::unix::SignalKind;

use crate::{
    cli::{Cli, Commands},
    config::{CompileConfig, Config, ExecutableConfig, InstanceConfig, InstanceType, ProcessesConfig, WebserverConfig},
};

mod cli;
mod config;
mod process_manager;
mod processes;
mod webserver;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::init();
    let config_path = cli.get_config_path();

    init_logger()?;

    match cli.commands {
        Commands::Init => {
            // if config_path.exists() {
            //     log::info!("Config file exists at {}", config_path.display());
            //     return Ok(());
            // }

            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let file = fs::File::create(&config_path)?;
            Config {
                base_dir: cli
                    .common
                    .base_dir
                    .or_else(|| config_path.parent().map(|p| p.to_path_buf()))
                    .unwrap_or_else(|| std::env::current_dir().unwrap())
                    .canonicalize()?,
                webserver: WebserverConfig::default(),
                processes: ProcessesConfig {
                    always_compile: false,
                    instances: vec![InstanceConfig {
                        name: "Minotari Nodes".to_string(),
                        instance_type: InstanceType::MinoTariNode,
                        num_instances: 1,
                    }],
                    executables: vec![ExecutableConfig {
                        instance_type: InstanceType::MinoTariNode,
                        // Default to compile.package_name(.exe)?
                        execuable_path: None,
                        compile: Some(CompileConfig {
                            working_dir: Some("../tari".into()),
                            package_name: "minotari_node".to_string(),
                            // Default is "{working_dir}/target/release"
                            target_dir: None,
                        }),
                        env: vec![],
                    }],
                },
            }
            .write(file)?;
            let config_path = config_path
                .canonicalize()
                .context("Failed to canonicalize config_path")?;
            log::info!("Config file created at {}", config_path.display());
        },
        Commands::Start => {
            start(&cli).await?;
        },
    }
    Ok(())
}

async fn start(cli: &Cli) -> anyhow::Result<()> {
    let config = Config::load_with_cli(cli)?;
    let _pid = lockfile::Lockfile::create(config.base_dir.join("tari_swarm.pid"))
        .context("Failed to acquire lockfile. Is another instance already running?")?;

    let shutdown = Shutdown::new();
    let signal = shutdown.to_signal().select(exit_signal()?);
    let webserver = webserver::spawn(config.webserver, shutdown.to_signal());
    let pm_handle = process_manager::spawn(config.base_dir.clone(), config.processes, shutdown.to_signal());

    tokio::select! {
        _ = signal => {
            log::info!("Shutting down...");
        },
        result = webserver => {
            result??;
            log::info!("Webserver exited");
        },
        result = pm_handle => {
            result??;
            log::info!("Process manager exited");
        }
    }

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
    let colors = fern::colors::ColoredLevelConfig::new().info(fern::colors::Color::Green);
    fern::Dispatch::new()
        .format(move |out, message, record| {
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
