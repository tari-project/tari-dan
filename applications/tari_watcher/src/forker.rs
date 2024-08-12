// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{net::IpAddr, path::PathBuf, process::Stdio};

use tokio::{
    fs,
    process::{Child, Command},
};

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

    pub async fn start_validator(
        &mut self,
        config: ExecutableConfig,
        base_node_grpc_address: String,
        vn_public_json_rpc_address: String,
        vn_gui_http_address: String,
    ) -> anyhow::Result<Child> {
        let instance = Instance::new(InstanceType::TariValidatorNode, config.clone());
        self.validator = Some(instance.clone());

        let mut command = self
            .get_command(
                config.executable_path.unwrap(),
                "esmeralda".to_string(), // TODO: add network to cfg
                base_node_grpc_address,
                vn_public_json_rpc_address,
                vn_gui_http_address,
            )
            .await?;

        // TODO: stdout logs
        // let process_dir = self.base_dir.join("processes").join("TariValidatorNode");
        // let stdout_log_path = process_dir.join("stdout.log");
        // let stderr_log_path = process_dir.join("stderr.log");
        command
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        let child = command.spawn()?;

        Ok(child)
    }

    async fn get_command(
        &self,
        target_binary: PathBuf,
        network: String,
        base_node_grpc_address: String,
        json_rpc_public_address: String,
        web_ui_address: String,
    ) -> anyhow::Result<Command> {
        log::debug!("Creating validator command from base directory: {:?}", self.base_dir);

        // create directory for the validator process
        let process_dir = self.base_dir.join("processes").join("TariValidatorNode");
        fs::create_dir_all(&process_dir).await?;

        log::debug!("Creating validator process to run from: {:?}", process_dir);

        let json_rpc_address = json_rpc_public_address.clone();
        let mut command = Command::new(target_binary);
        let empty: Vec<(&str, &str)> = Vec::new();
        command
            .envs(empty)
            .arg("-b")
            .arg(process_dir)
            .arg("--network")
            .arg(network)
            .arg(format!("--json-rpc-public-address={json_rpc_public_address}"))
            .arg(format!(
                "-pvalidator_node.base_node_grpc_address={base_node_grpc_address}"
            ))
            .arg(format!("-pvalidator_node.json_rpc_listener_address={json_rpc_address}"))
            .arg(format!("-pvalidator_node.http_ui_listener_address={web_ui_address}"))
            .arg("-pvalidator_node.base_layer_scanning_interval=1");
        Ok(command)
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
            config: config.clone(),
            listen_ip: None,
            port: PortAllocator::new(),
        }
    }
}
