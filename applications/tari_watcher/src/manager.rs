// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_shutdown::ShutdownSignal;

use crate::{
    config::{Config, ExecutableConfig},
    forker::Forker,
};

pub struct ProcessManager {
    pub validator_config: ExecutableConfig,
    pub wallet_config: ExecutableConfig,
    pub forker: Forker,
    pub shutdown_signal: ShutdownSignal,
}

impl ProcessManager {
    pub fn new(config: Config, shutdown_signal: ShutdownSignal) -> Self {
        Self {
            validator_config: config.executable_config[0].clone(),
            wallet_config: config.executable_config[1].clone(),
            forker: Forker::new(config.base_node_grpc_address, config.base_dir),
            shutdown_signal,
        }
    }

    pub async fn start(mut self) -> anyhow::Result<()> {
        info!("Starting validator node process");
        self.forker.start_validator(self.validator_config.clone()).await?;

        loop {
            tokio::select! {
                _ = self.shutdown_signal.wait() => {
                    info!("Shutting down process manager");
                    break;
                }
            }
        }

        Ok(())
    }
}
