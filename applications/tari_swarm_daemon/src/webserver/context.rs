//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::{config::Config, process_manager::ProcessManagerHandle};
use std::sync::Arc;
use tari_shutdown::Shutdown;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct HandlerContext {
    config: Config,
    pm_handle: ProcessManagerHandle,
    mining_shutdown: Arc<RwLock<Option<Shutdown>>>,
}

impl HandlerContext {
    pub fn new(config: Config, pm_handle: ProcessManagerHandle) -> Self {
        Self { config, pm_handle, mining_shutdown: Arc::new(RwLock::new(None)) }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn process_manager(&self) -> &ProcessManagerHandle {
        &self.pm_handle
    }

    pub fn change_mining_shutdown(&self, shutdown: Shutdown) {
        // TODO: continue
    }
}
