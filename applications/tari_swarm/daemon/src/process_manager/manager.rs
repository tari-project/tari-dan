//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{path::PathBuf, time::Duration};

use tokio::time;

use crate::{
    config::ProcessesConfig,
    process_manager::{executables::ExecutableManager, instances::InstanceManager},
};

pub struct ProcessManager {
    executable_manager: ExecutableManager,
    instance_manager: InstanceManager,
}

impl ProcessManager {
    pub fn new(base_dir: PathBuf, config: ProcessesConfig) -> Self {
        Self {
            executable_manager: ExecutableManager::new(config.executables, config.always_compile),
            instance_manager: InstanceManager::new(base_dir, config.instances),
        }
    }

    pub async fn start(&mut self) -> anyhow::Result<()> {
        log::info!("Starting process manager");
        let executables = self.executable_manager.prepare().await?;
        self.instance_manager.fork_all(executables).await?;

        loop {
            // TODO
            time::sleep(Duration::from_secs(5)).await;
        }

        #[allow(unreachable_code)]
        Ok(())
    }
}
