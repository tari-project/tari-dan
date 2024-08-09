// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use crate::config::{Config, ExecutableConfig};
use crate::forker::Forker;

pub struct ProcessManager {
    pub validator_config: ExecutableConfig,
    pub wallet_config: ExecutableConfig,
    pub forker: Forker,
}

impl ProcessManager {
    pub fn new(config: Config) -> Self {
        Self {
            validator_config: config.executable_config[0].clone(),
            wallet_config: config.executable_config[1].clone(),
            forker: Forker::new(config.base_node_grpc_address, config.base_dir),
        }
    }
}
