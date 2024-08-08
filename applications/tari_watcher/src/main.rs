// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use anyhow::{anyhow, Context};
use tokio::fs;

use crate::{
    cli::{Cli, Commands},
    config::get_base_config,
};

mod cli;
mod config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::init();
    let config_path = cli.get_config_path();

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

            // TODO: use standardised logging
            // if let Err(e) = initialize_logging(..)
            log::info!("Config file created at {}", config_path.display());
        },
        Commands::Start(ref args) => {
            let mut config = get_base_config(&cli)?;
            // optionally override config values
            args.apply(&mut config);

            unimplemented!("Start command not implemented");
        },
    }

    Ok(())
}
