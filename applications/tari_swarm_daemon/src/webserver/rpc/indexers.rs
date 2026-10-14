//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{config::InstanceType, process_manager::InstanceId, webserver::context::HandlerContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListIndexersRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListIndexersResponse {
    pub nodes: Vec<IndexerInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerInfo {
    pub instance_id: InstanceId,
    pub name: String,
    pub web: Url,
    pub api_url: Url,
    pub is_running: bool,
}

pub async fn list(context: &HandlerContext, _req: ListIndexersRequest) -> Result<ListIndexersResponse, anyhow::Error> {
    let instances = context.process_manager().list_indexers().await?;

    let nodes = instances
        .into_iter()
        .map(|instance| {
            let api_url = instance.get_public_api_url();
            let web = instance.get_public_web_url();

            Ok(IndexerInfo {
                instance_id: instance.id,
                name: instance.name,
                web,
                api_url,
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
