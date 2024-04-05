//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use tokio::task;

use crate::config::ProcessesConfig;

mod executables;
mod instances;
mod manager;
mod port_allocator;
mod utils;

pub use instances::InstanceId;
pub use port_allocator::*;

pub fn spawn(
    base_dir: PathBuf,
    config: ProcessesConfig,
    shutdown: tari_shutdown::ShutdownSignal,
) -> task::JoinHandle<anyhow::Result<()>> {
    tokio::spawn(async {
        let mut manager = manager::ProcessManager::new(base_dir, config);
        tokio::select! {
            _ = shutdown => Ok(()),
            result = manager.start() => {
                result
            }
        }
    })
}
