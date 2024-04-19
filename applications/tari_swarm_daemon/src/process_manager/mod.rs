//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tokio::task;

use crate::config::Config;

mod executables;
mod handle;

mod instances;
pub use instances::*;
mod manager;
mod port_allocator;
mod processes;
pub use handle::*;
pub use instances::InstanceId;
pub use port_allocator::*;
pub use processes::*;

pub fn spawn(
    config: &Config,
    shutdown: tari_shutdown::ShutdownSignal,
) -> (task::JoinHandle<anyhow::Result<()>>, ProcessManagerHandle) {
    let (manager, handle) = manager::ProcessManager::new(config, shutdown);

    let task_handle = tokio::spawn(manager.start());

    (task_handle, handle)
}
