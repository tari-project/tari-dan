// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use clap::Parser;

use crate::config::{Config, InstanceType};

#[derive(Clone, Debug, Parser)]
pub struct Cli {
    #[clap(flatten)]
    pub common: CommonCli,
    #[clap(subcommand)]
    pub command: Commands,
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
    #[clap(short = 'c', long, parse(from_os_str), default_value = "./data/watcher/config.toml")]
    pub config_path: PathBuf,
}

#[derive(Clone, Debug, clap::Subcommand)]
pub enum Commands {
    Init(InitArgs),
    Start(Overrides),
}

#[derive(Clone, Debug, clap::Args)]
pub struct InitArgs {
    #[clap(long)]
    /// Disable initial and auto registration of the validator node
    pub no_auto_register: bool,
}

impl InitArgs {
    pub fn apply(&self, config: &mut Config) {
        config.auto_register = !self.no_auto_register;
    }
}

#[derive(Clone, Debug, clap::Args)]
pub struct Overrides {
    #[clap(long)]
    // The path to the validator node binary (optional)
    pub vn_node_path: Option<PathBuf>,
}

impl Overrides {
    pub fn apply(&self, config: &mut Config) {
        if self.vn_node_path.is_none() {
            return;
        }

        if let Some(exec_config) = config
            .executable_config
            .iter_mut()
            .find(|c| c.instance_type == InstanceType::TariValidatorNode)
        {
            exec_config.executable_path = self.vn_node_path.clone();
        }
        log::info!(
            "Overriding validator node binary path to {:?}",
            self.vn_node_path.as_ref().unwrap()
        );
    }
}
