// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{
    env,
    net::IpAddr,
    path::{Path, PathBuf},
    process::Stdio,
};

use tokio::process::{Child, Command};

use crate::{
    config::{ExecutableConfig, InstanceType},
    port::PortAllocator,
};

#[allow(dead_code)]
pub struct Forker {
    // Used for the validator to connect to the base (L1) node
    base_node_grpc_address: String,
    // The base directory of calling the application
    base_dir: PathBuf,
    // The Tari L2 validator instance
    validator: Option<Instance>,
    // The Minotari L1 wallet instance
    wallet: Option<Instance>,
}

impl Forker {
    pub fn new(base_node_grpc_address: String, base_dir: PathBuf) -> Self {
        Self {
            validator: None,
            wallet: None,
            base_node_grpc_address,
            base_dir,
        }
    }

    pub async fn start_validator(&mut self, config: ExecutableConfig) -> anyhow::Result<Child> {
        let instance = Instance::new(InstanceType::TariValidatorNode, config.clone());
        self.validator = Some(instance.clone());

        let mut cmd = Command::new(
            config
                .executable_path
                .unwrap_or_else(|| Path::new("tari_validator_node").to_path_buf()),
        );

        // TODO: stdout logs
        // let process_dir = self.base_dir.join("processes").join("TariValidatorNode");
        // let stdout_log_path = process_dir.join("stdout.log");
        // let stderr_log_path = process_dir.join("stderr.log");
        cmd.envs(env::vars())
            //.arg(format!("--config={validator_node_config_path}"))
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        let child = cmd.spawn()?;

        Ok(child)
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct Instance {
    app: InstanceType,
    config: ExecutableConfig,
    listen_ip: Option<IpAddr>,
    port: PortAllocator,
}

impl Instance {
    pub fn new(app: InstanceType, config: ExecutableConfig) -> Self {
        Self {
            app,
            config,
            listen_ip: None,
            port: PortAllocator::new(),
        }
    }
}
