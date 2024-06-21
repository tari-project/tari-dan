//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use anyhow::anyhow;
use async_trait::async_trait;
use log::debug;
use tokio::process::Command;

use crate::process_definitions::{ProcessContext, ProcessDefinition};

#[derive(Debug, Default)]
pub struct ValidatorNode;

impl ValidatorNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProcessDefinition for ValidatorNode {
    async fn get_command(&self, mut context: ProcessContext<'_>) -> anyhow::Result<Command> {
        let mut command = Command::new(context.bin());
        let jrpc_port = context.get_free_port("jrpc").await?;
        let web_ui_port = context.get_free_port("web").await?;
        let local_ip = context.local_ip();

        let json_rpc_public_address = format!("{local_ip}:{jrpc_port}");
        let json_rpc_address = format!("{local_ip}:{jrpc_port}");
        let web_ui_address = format!("{local_ip}:{web_ui_port}");

        let base_node = context
            .minotari_nodes()
            .next()
            .ok_or_else(|| anyhow!("Base nodes should be started before validator nodes"))?;

        let base_node_grpc_address = base_node
            .instance()
            .allocated_ports()
            .get("grpc")
            .map(|port| format!("{local_ip}:{port}"))
            .ok_or_else(|| anyhow!("grpc port not found for base node"))?;

        debug!(
            "Starting validator node #{} with base node grpc address: {}",
            context.instance_id(),
            base_node_grpc_address
        );

        command
            .envs(context.environment())
            .arg("-b")
            .arg(context.base_path())
            .arg("--network")
            .arg(context.network().to_string())
            .arg(format!("--json-rpc-public-address={json_rpc_public_address}"))
            .arg(format!(
                "-pvalidator_node.base_node_grpc_address={base_node_grpc_address}"
            ))
            .arg(format!("-pvalidator_node.json_rpc_listener_address={json_rpc_address}"))
            .arg(format!("-pvalidator_node.http_ui_listener_address={web_ui_address}"))
            .arg("-pvalidator_node.base_layer_scanning_interval=1");

        Ok(command)
    }

    fn get_relative_data_path(&self) -> Option<PathBuf> {
        Some("data".into())
    }
}
