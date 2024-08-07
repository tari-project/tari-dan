// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use crate::cli::{Cli, Commands};
use anyhow::{anyhow, Context};
use tokio::fs;

use crate::config::get_base_config;

mod cli;
mod config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::init();
    let config_path = cli.get_config_path();

    match cli.command {
        Commands::Init => {
            // set by default in CommonCli
            let parent = config_path.parent().unwrap();
            fs::create_dir_all(parent).await?;

            let config = get_base_config(&cli)?;

            let file = fs::File::create(&config_path)
                .await
                .with_context(|| anyhow!("Failed to open config path {}", config_path.display()))?;
            config.write(file).await.context("Writing config failed")?;

            let config_path = config_path
                .canonicalize()
                .context("Failed to canonicalize config path")?;
            log::info!("Config file created at {}", config_path.display());
        },
        Commands::Start => {
            unimplemented!("Start command not implemented");
        },
    }

    Ok(())
}
