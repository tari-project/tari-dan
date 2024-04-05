//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod instance;

use std::{
    collections::HashMap,
    io::Write,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    process::Stdio,
};

use anyhow::anyhow;
pub use instance::InstanceId;
use tari_common::configuration::Network;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    task,
};

use super::{executables::Executables, instances::instance::Instance};
use crate::{
    config::{InstanceConfig, InstanceType},
    process_manager::{executables::Executable, port_allocator::PortAllocator},
    processes::{get_definition, ProcessContext, ProcessDefinition},
};

pub struct InstanceManager {
    base_path: PathBuf,
    config: Vec<InstanceConfig>,
    _running_instances: HashMap<InstanceType, Vec<Instance>>,
    port_allocator: PortAllocator,
    instance_id: InstanceId,
}

impl InstanceManager {
    pub fn new(base_path: PathBuf, config: Vec<InstanceConfig>) -> Self {
        Self {
            base_path,
            config,
            _running_instances: HashMap::new(),
            port_allocator: PortAllocator::new(),
            instance_id: 0,
        }
    }

    /// Fork all defined processes in order
    pub async fn fork_all(&mut self, executables: Executables<'_>) -> anyhow::Result<()> {
        for instance in self.config.clone() {
            let executable = executables.get(instance.instance_type).ok_or_else(|| {
                anyhow!(
                    "No executable found for instance type '{}'. This is a bug in the configuration",
                    instance.instance_type
                )
            })?;

            let definition = get_definition(instance.instance_type);
            self.fork(executable, definition, instance).await?;
        }
        Ok(())
    }

    async fn fork(
        &mut self,
        executable: &Executable,
        definition: Box<dyn ProcessDefinition>,
        instance: InstanceConfig,
    ) -> anyhow::Result<()> {
        let local_ip = IpAddr::V4(Ipv4Addr::from([127, 0, 0, 1]));

        for _ in 0..instance.num_instances {
            let instance_id = self.next_instance_id();
            log::info!(
                "Starting {} (id: {}, path: {})",
                instance.instance_type,
                instance_id,
                executable.path.display()
            );
            let context = ProcessContext::new(
                instance_id,
                &executable.path,
                self.base_path.clone(),
                Network::LocalNet,
                local_ip,
                &mut self.port_allocator,
            );
            let mut command = definition.get_command(context).await?;
            command
                .kill_on_drop(true)
                // .stdout(Stdio::piped())
                // .stderr(Stdio::piped())
                // Any attempt to use stdin will fail immediately
                .stdin(Stdio::null());
            log::debug!("Starting {} (command: {:?})", instance.instance_type, command);
            let mut child = command.spawn()?;

            if let Some(stdout) = child.stdout.take().map(BufReader::new) {
                task::spawn(async move {
                    let mut lines = stdout.lines();
                    while let Some(output) = lines.next_line().await.unwrap() {
                        eprintln!("{output}");
                        std::io::stderr().flush().unwrap();
                    }
                    log::debug!("EXIT {} READING STDOUT", instance.instance_type);
                });
            }
            if let Some(stderr) = child.stderr.take().map(BufReader::new) {
                task::spawn(async move {
                    let mut lines = stderr.lines();
                    while let Some(output) = lines.next_line().await.unwrap() {
                        eprintln!("{output}");
                        std::io::stderr().flush().unwrap();
                    }
                    log::debug!("EXIT {} READING STDerr", instance.instance_type);
                });
            }

            // TODO: keep child in self.instances
            task::spawn(async move { child.wait().await });
        }
        Ok(())
    }

    fn next_instance_id(&mut self) -> InstanceId {
        let id = self.instance_id;
        self.instance_id += 1;
        id
    }
}
