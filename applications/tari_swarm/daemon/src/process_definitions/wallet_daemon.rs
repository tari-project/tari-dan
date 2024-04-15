//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::process::Command;

use crate::process_definitions::{ProcessContext, ProcessDefinition};

#[derive(Debug, Default)]
pub struct WalletDaemon;

impl WalletDaemon {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProcessDefinition for WalletDaemon {
    async fn get_command(&self, mut context: ProcessContext<'_>) -> anyhow::Result<Command> {
        let mut command = Command::new(context.bin());
        let jrpc_port = context.get_free_port("jrpc").await?;
        let web_ui_port = context.get_free_port("web").await?;
        let local_ip = context.local_ip();

        let json_rpc_public_address = format!("{local_ip}:{jrpc_port}");
        let json_rpc_address = format!("{local_ip}:{jrpc_port}");
        let web_ui_address = format!("{local_ip}:{web_ui_port}");

        let indexer = context
            .get_indexers()
            .next()
            .ok_or_else(|| anyhow!("Indexer should be started before wallet daemon"))?;
        let indexer_url = format!(
            "http://{local_ip}:{}",
            indexer
                .instance()
                .allocated_ports()
                .get("jrpc")
                .ok_or_else(|| anyhow!("Indexer jrpc port not found"))?
        );

        command
            .envs(context.environment())
            .arg("-b")
            .arg(context.base_path())
            .arg("--network")
            .arg(context.network().to_string())
            .arg(format!("--json-rpc-address={json_rpc_address}"))
            .arg(format!("--indexer-url={indexer_url}"))
            .arg(format!("--ui-connect-address={json_rpc_public_address}"))
            .arg(format!("-pdan_wallet_daemon.http_ui_address={web_ui_address}"));

        Ok(command)
    }
}
