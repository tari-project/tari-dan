//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::process::Command;

use crate::process_definitions::{ARGS_SETTINGS_KEY, ProcessContext, ProcessDefinition};

#[derive(Debug, Default)]
pub struct Indexer;

impl Indexer {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProcessDefinition for Indexer {
    async fn get_command(&self, mut context: ProcessContext<'_>) -> anyhow::Result<Command> {
        let mut command = Command::new(context.bin());
        let api_port = context.get_free_port("api").await?;
        let web_ui_port = context.get_free_port("web").await?;
        let listen_ip = context.listen_ip();

        let api_public_url = context.get_public_api_url();
        let api_listener_address = format!("{listen_ip}:{api_port}");
        let web_ui_listener_address = format!("{listen_ip}:{web_ui_port}");

        let rate_limit = context
            .get_setting("enable_rate_limit")
            .map(|s| s == "true")
            .unwrap_or(false);

        let base_node = context
            .minotari_nodes()
            .next()
            .ok_or_else(|| anyhow!("Base nodes should be started before validator nodes"))?;

        let base_node_grpc_url = base_node
            .instance()
            .allocated_ports()
            .get("grpc")
            .map(|port| format!("http://127.0.0.1:{port}"))
            .ok_or_else(|| anyhow!("grpc port not found for base node"))?;

        command
            .envs(context.environment())
            .arg("-b")
            .arg(context.base_path())
            .arg("--network")
            .arg(context.network().to_string())
            .arg(format!(
                "-pepoch_oracle.base_layer.base_node_grpc_url={base_node_grpc_url}"
            ))
            .arg(format!("-pindexer.api_listen_address={api_listener_address}"))
            .arg(format!("-pindexer.web_ui_address={web_ui_listener_address}"))
            .arg(format!("-pindexer.web_ui_public_api_url={api_public_url}"))
            .arg("-pepoch_oracle.base_layer.scanning_interval=1")
            .arg(format!("-pindexer.rate_limits.enabled={rate_limit}"))
            .arg(format!(
                "-pindexer.localnet_consensus_constants_file={}",
                context.consensus_constants_file().display()
            ))
            .env("API_DEBUG", "1"); // Enable detailed API error messages

        // mDNS is not guaranteed to work, so we'll explicitly set the seed peers for the indexer.
        let seed_peers = context.validator_nodes().filter_map(|vn| {
            let port = vn.instance().allocated_ports().get("p2p")?;
            Some(format!("/ip4/127.0.0.1/tcp/{port}"))
        });

        for seed in seed_peers {
            command.arg(format!("--peer-seeds={seed}"));
        }

        if let Some(args) = context.get_setting(ARGS_SETTINGS_KEY) {
            for arg in args.split_whitespace() {
                command.arg(arg);
            }
        }

        Ok(command)
    }

    fn get_relative_data_path(&self) -> Option<PathBuf> {
        Some("data".into())
    }
}
