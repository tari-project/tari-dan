//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use crate::{config::InstanceType, process_manager::InstanceId, webserver::context::HandlerContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListIndexersRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListIndexersResponse {
    pub nodes: Vec<ValidatorNodeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorNodeInfo {
    pub name: String,
    pub web: String,
    pub jrpc: String,
    pub is_running: bool,
}

pub async fn list(context: &HandlerContext, _req: ListIndexersRequest) -> Result<ListIndexersResponse, anyhow::Error> {
    let instances = context.process_manager().list_indexers().await?;

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

            Ok(ValidatorNodeInfo {
                name: instance.name,
                web,
                jrpc,
                is_running: instance.is_running,
            })
        })
        .collect::<anyhow::Result<_>>()?;

    Ok(ListIndexersResponse { nodes })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerCreateRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerCreateResponse {
    pub instance_id: InstanceId,
}

pub async fn create(
    context: &HandlerContext,
    req: IndexerCreateRequest,
) -> Result<IndexerCreateResponse, anyhow::Error> {
    let instance_id = context
        .process_manager()
        .create_instance(req.name, InstanceType::TariIndexer, HashMap::new())
        .await?;

    Ok(IndexerCreateResponse { instance_id })
}
