//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::config::Config;
use crate::manager::ProcessManager;
use tari_shutdown::ShutdownSignal;
use tokio::task;

pub fn spawn(config: Config, shutdown: ShutdownSignal) -> task::JoinHandle<anyhow::Result<()>> {
    let manager = ProcessManager::new(config, shutdown);
    let task_handle = tokio::spawn(manager.start());
    task_handle
}
