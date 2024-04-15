//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use log::debug;
use tokio::process::Command;

use crate::process_definitions::{ProcessContext, ProcessDefinition};

#[derive(Debug, Default)]
pub struct MinotariNode;

impl MinotariNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProcessDefinition for MinotariNode {
    async fn get_command(&self, mut context: ProcessContext<'_>) -> anyhow::Result<Command> {
        let mut command = Command::new(context.bin());
        let p2p_port = context.get_free_port("p2p").await?;
        let grpc_port = context.get_free_port("grpc").await?;
        let local_ip = context.local_ip();

        let public_address = format!("/ip4/{local_ip}/tcp/{p2p_port}");

        let base_nodes = context.get_minotari_nodes();
        let mut base_node_addresses = Vec::new();
        for base_node in base_nodes {
            let identity = base_node.get_identity().await?;
            debug!("Base node identity: {identity}");
            base_node_addresses.push(identity);
        }

        command
            .envs(context.environment())
            .arg("-b")
            .arg(context.base_path())
            .arg("--network")
            .arg(context.network().to_string())
            .arg("-pbase_node.p2p.transport.type=tcp")
            .arg(format!(
                "-pbase_node.p2p.transport.tcp.listener_address={public_address}"
            ))
            .arg(format!("-pbase_node.p2p.public_addresses={public_address}"))
            .arg(format!("-pbase_node.grpc_address=/ip4/{local_ip}/tcp/{grpc_port}"))
            .args([
                "--non-interactive",
                "--enable-grpc",
                "--enable-mining",
                "--enable-second-layer",
                "-pbase_node.p2p.allow_test_addresses=true",
                "-pbase_node.metadata_auto_ping_interval=3",
                "-pbase_node.report_grpc_error=true",
            ]);
        // TODO: seed nodes?
        // # "-p",
        // # f'{NETWORK}.p2p.seeds.peer_seeds="369ae9a89c3fc2804d6ec07e20bf10e5d0e72f565a71821fc7c611ae5bee0116::/ip4/
        // 34.252.174.111/tcp/18000"',

        Ok(command)
    }

    // fn get_relative_data_path(&self) -> Option<PathBuf> {
    //     Some(Path::new("network/data"))
    // }
}
