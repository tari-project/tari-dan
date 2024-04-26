//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tokio::process::Command;

use crate::process_definitions::{ProcessContext, ProcessDefinition};

#[derive(Debug, Default)]
pub struct SignalingServer;

impl SignalingServer {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProcessDefinition for SignalingServer {
    async fn get_command(&self, mut context: ProcessContext<'_>) -> anyhow::Result<Command> {
        let mut command = Command::new(context.bin());
        let jrpc_port = context.get_free_port("jrpc").await?;
        let local_ip = context.local_ip();
        let listen_addr = format!("{local_ip}:{jrpc_port}");

        command
            .arg("-b")
            .arg(context.base_path())
            .arg(format!("--listen-addr={listen_addr}"));

        Ok(command)
    }
}
