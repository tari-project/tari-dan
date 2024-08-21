//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{net::SocketAddr, path::PathBuf};

use anyhow::Context;
use clap::Parser;
use tari_common::configuration::Network;

use crate::config::{Config, InstanceType};

#[derive(Debug, Clone, Parser)]
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
    #[clap(short = 'c', long, parse(from_os_str), default_value = "./config.toml")]
    pub config_path: PathBuf,
    #[clap(short = 'n', long)]
    pub network: Option<Network>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Commands {
    Init(InitArgs),
    Start(Overrides),
}

#[derive(Debug, Clone, clap::Args)]
pub struct InitArgs {
    /// Overwrite the config file even if it exists
    #[clap(long)]
    pub force: bool,
    #[clap(flatten)]
    pub overrides: Overrides,
}

#[derive(Debug, Clone, clap::Args)]
pub struct Overrides {
    #[clap(long, env = "TARI_SWARM_WEBUI_LISTEN_ADDRESS")]
    pub webui_listen_address: Option<SocketAddr>,
    #[clap(long)]
    pub no_compile: bool,
    #[clap(long)]
    pub binaries_root: Option<PathBuf>,
    #[clap(long)]
    pub start_port: Option<u16>,
    #[clap(short = 'k', long)]
    pub skip_registration: bool,
}

impl Overrides {
    pub fn apply(&self, config: &mut Config) -> anyhow::Result<()> {
        for exec_mut in &mut config.processes.executables {
            if let Some(ref root) = self.binaries_root {
                let package = exec_mut
                    .compile
                    .as_ref()
                    .map(|c| c.package_name.clone())
                    .unwrap_or_else(|| instance_type_to_package_name(exec_mut.instance_type));
                exec_mut.execuable_path = Some(
                    root.canonicalize()
                        .context("Root override path does not exist")?
                        .join(package),
                );
            }
            if self.no_compile {
                exec_mut.compile = None;
            }
        }

        if self.no_compile {
            config.processes.force_compile = false;
        }

        if let Some(listen_addr) = self.webui_listen_address {
            config.webserver.bind_address = listen_addr;
        }

        if let Some(port) = self.start_port {
            config.start_port = port;
        }

        Ok(())
    }
}

fn instance_type_to_package_name(instance_type: InstanceType) -> String {
    match instance_type {
        InstanceType::MinoTariNode => "minotari_node".to_string(),
        InstanceType::MinoTariConsoleWallet => "minotari_console_wallet".to_string(),
        InstanceType::MinoTariMiner => "minotari_miner".to_string(),
        InstanceType::TariValidatorNode => "tari_validator_node".to_string(),
        InstanceType::TariIndexer => "tari_indexer".to_string(),
        InstanceType::TariWalletDaemon => "tari_dan_wallet_daemon".to_string(),
        InstanceType::TariSignalingServer => "tari_signaling_server".to_string(),
    }
}
