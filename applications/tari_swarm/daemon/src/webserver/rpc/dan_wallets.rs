//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use crate::webserver::context::HandlerContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDanWalletsRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDanWalletsResponse {
    pub nodes: Vec<DanWalletInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DanWalletInfo {
    pub name: String,
    pub web: String,
    pub jrpc: String,
    pub is_running: bool,
}

pub async fn list(
    context: &HandlerContext,
    _req: ListDanWalletsRequest,
) -> Result<ListDanWalletsResponse, anyhow::Error> {
    let instances = context.process_manager().list_wallet_daemons().await?;

    let nodes = instances
        .into_iter()
        .map(|instance| {
            let web_port = instance.ports.get("web").ok_or_else(|| anyhow!("web port not found"))?;
            let json_rpc_port = instance
                .ports
                .get("jrpc")
                .ok_or_else(|| anyhow!("jrpc port not found"))?;
            let web = format!("http://localhost:{web_port}");
            let jrpc = format!("http://localhost:{json_rpc_port}");

            Ok(DanWalletInfo {
                name: instance.name,
                web,
                jrpc,
                // TODO
                // is_running: status == InstanceStatus::Running
                is_running: true,
            })
        })
        .collect::<anyhow::Result<_>>()?;

    Ok(ListDanWalletsResponse { nodes })
}
