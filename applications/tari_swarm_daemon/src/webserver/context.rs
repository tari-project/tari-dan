//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use anyhow::bail;
use tari_shutdown::Shutdown;
use tokio::sync::RwLock;

use crate::{config::Config, process_manager::ProcessManagerHandle};

#[derive(Debug, Clone)]
pub struct HandlerContext {
    config: Config,
    pm_handle: ProcessManagerHandle,
    mining_shutdown: Arc<RwLock<Option<Shutdown>>>,
}

impl HandlerContext {
    pub fn new(config: Config, pm_handle: ProcessManagerHandle) -> Self {
        Self {
            config,
            pm_handle,
            mining_shutdown: Arc::new(RwLock::new(None)),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn process_manager(&self) -> &ProcessManagerHandle {
        &self.pm_handle
    }

    pub async fn start_mining(&self, shutdown: Shutdown) -> anyhow::Result<()> {
        let lock = self.mining_shutdown.read().await;
        if lock.is_none() || lock.as_ref().is_some_and(|curr_shutdown| curr_shutdown.is_triggered()) {
            drop(lock);
            let mut lock = self.mining_shutdown.write().await;
            if lock.is_none() || lock.as_ref().is_some_and(|curr_shutdown| curr_shutdown.is_triggered()) {
                *lock = Some(shutdown);
                return Ok(());
            }
        }

        bail!("Mining already running!")
    }

    pub async fn stop_mining(&self) {
        let mut lock = self.mining_shutdown.write().await;
        if let Some(curr_shutdown) = lock.as_mut() {
            curr_shutdown.trigger();
        }
        *lock = None;
    }

    pub async fn is_mining(&self) -> bool {
        let lock = self.mining_shutdown.read().await;
        lock.as_ref().is_some_and(|curr_shutdown| !curr_shutdown.is_triggered())
    }
}
