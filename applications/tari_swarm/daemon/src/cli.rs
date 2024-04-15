//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use clap::Parser;
use tari_common::configuration::Network;

#[derive(Debug, Clone, Parser)]
pub struct Cli {
    #[clap(flatten)]
    pub common: CommonCli,
    #[clap(subcommand)]
    pub commands: Commands,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }

    pub fn get_config_path(&self) -> PathBuf {
        let Some(ref base_dir) = self.common.base_dir else {
            return self.common.config_path.clone();
        };
        if self.common.config_path.is_relative() {
            base_dir.join(&self.common.config_path)
        } else {
            self.common.config_path.clone()
        }
    }
}

#[derive(Debug, Clone, clap::Args)]
pub struct CommonCli {
    #[clap(short = 'b', long, parse(from_os_str))]
    pub base_dir: Option<PathBuf>,
    #[clap(short = 'c', long, parse(from_os_str), default_value = "config.toml")]
    pub config_path: PathBuf,
    #[clap(short = 'n', long)]
    pub network: Option<Network>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Commands {
    Init,
    Start,
    // #[clap(name = "stop")]
    // Stop(Stop),
}
