// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::time::SystemTime;

use anyhow::{anyhow, Context};
use tokio::fs;

use crate::{
    cli::{Cli, Commands},
    config::{get_base_config, Config},
    manager::ProcessManager,
};

mod cli;
mod config;
mod forker;
mod manager;
mod port;

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
            let mut config = get_base_config(&cli)?;
            // optionally override config values
            args.apply(&mut config);
            start(config).await?;
        },
    }

    Ok(())
}

async fn start(config: Config) -> anyhow::Result<()> {
    let mut manager = ProcessManager::new(config.clone());
    manager.forker.start_validator(manager.validator_config).await?;

    Ok(())
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
