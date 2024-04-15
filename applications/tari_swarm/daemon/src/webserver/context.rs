//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common::configuration::Network;

use crate::{config::Config, process_manager::ProcessManagerHandle};

#[derive(Debug, Clone)]
pub struct HandlerContext {
    config: Config,
    pm_handle: ProcessManagerHandle,
}

impl HandlerContext {
    pub fn new(config: Config, pm_handle: ProcessManagerHandle) -> Self {
        Self { config, pm_handle }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn process_manager(&self) -> &ProcessManagerHandle {
        &self.pm_handle
    }

    pub fn network(&self) -> Network {
        self.config.network
    }
}
