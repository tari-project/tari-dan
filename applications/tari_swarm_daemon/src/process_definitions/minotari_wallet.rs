//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use anyhow::anyhow;
use async_trait::async_trait;
use log::*;
use tokio::process::Command;

use crate::process_definitions::{ProcessContext, ProcessDefinition};

#[derive(Debug, Default)]
pub struct MinotariWallet;

impl MinotariWallet {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProcessDefinition for MinotariWallet {
    async fn get_command(&self, mut context: ProcessContext<'_>) -> anyhow::Result<Command> {
        let mut command = Command::new(context.bin());
        let p2p_port = context.get_free_port("p2p").await?;
        let grpc_port = context.get_free_port("grpc").await?;
        let local_ip = context.local_ip();

        let public_address = format!("/ip4/{local_ip}/tcp/{p2p_port}");

        let base_nodes = context.minotari_nodes().collect::<Vec<_>>();

        if base_nodes.is_empty() {
            return Err(anyhow!("Base nodes should be started before the console wallet"));
        }

        let mut base_node_addresses = Vec::with_capacity(base_nodes.len());
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
            .arg("--enable-grpc")
            .arg("--password")
            .arg("password")
            .arg(format!("-pwallet.custom_base_node={}", base_node_addresses[0]))
            .arg("-pwallet.p2p.transport.type=tcp")
            .arg(format!("-pwallet.p2p.transport.tcp.listener_address={public_address}"))
            .arg(format!("-pwallet.p2p.public_addresses={public_address}"))
            .arg(format!("-pwallet.grpc_address=/ip4/{local_ip}/tcp/{grpc_port}"))
            .args(["--non-interactive", "-pwallet.p2p.allow_test_addresses=true"])
            .arg(format!(
                "-p{}.p2p.seeds.peer_seeds={}",
                context.network(),
                base_node_addresses.join(",")
            ));

        debug!("Command: {:?}", command);

        Ok(command)
    }
}
