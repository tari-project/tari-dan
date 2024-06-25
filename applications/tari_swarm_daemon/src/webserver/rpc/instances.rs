//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, path::PathBuf};

use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};
use serde::{Deserialize, Serialize};

use crate::{config::InstanceType, process_manager::InstanceId, webserver::context::HandlerContext};

pub type StartInstanceRequest = String;

#[derive(Debug, Clone, Serialize)]
pub struct StartInstanceResponse {
    pub success: bool,
}

pub async fn start(
    context: &HandlerContext,
    req: StartInstanceRequest,
) -> Result<StartInstanceResponse, anyhow::Error> {
    let name = req;

    let instance = context
        .process_manager()
        .get_instance_by_name(name)
        .await?
        .ok_or_else(|| {
            JsonRpcError::new(
                JsonRpcErrorReason::ApplicationError(404),
                "Instance not found".to_string(),
                serde_json::Value::Null,
            )
        })?;

    context.process_manager().start_instance(instance.id).await?;

    Ok(StartInstanceResponse { success: true })
}

pub type StopInstanceRequest = String;

#[derive(Debug, Clone, Serialize)]
pub struct StopInstanceResponse {
    pub success: bool,
}

pub async fn stop(context: &HandlerContext, req: StopInstanceRequest) -> Result<StopInstanceResponse, anyhow::Error> {
    let name = req;

    let instance = context
        .process_manager()
        .get_instance_by_name(name)
        .await?
        .ok_or_else(|| {
            JsonRpcError::new(
                JsonRpcErrorReason::ApplicationError(404),
                "Instance not found".to_string(),
                serde_json::Value::Null,
            )
        })?;

    context.process_manager().stop_instance(instance.id).await?;

    Ok(StopInstanceResponse { success: true })
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListInstancesRequest {
    pub by_type: Option<InstanceType>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListInstancesResponse {
    pub instances: Vec<InstanceInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceInfo {
    pub id: InstanceId,
    pub name: String,
    pub ports: HashMap<&'static str, u16>,
    pub base_path: PathBuf,
    pub instance_type: InstanceType,
    pub is_running: bool,
}

impl From<crate::process_manager::InstanceInfo> for InstanceInfo {
    fn from(value: crate::process_manager::InstanceInfo) -> Self {
        Self {
            id: value.id,
            name: value.name,
            ports: value.ports.into_ports(),
            base_path: value.base_path,
            instance_type: value.instance_type,
            is_running: value.is_running,
        }
    }
}

pub async fn list(context: &HandlerContext, req: ListInstancesRequest) -> Result<ListInstancesResponse, anyhow::Error> {
    let instances = context.process_manager().list_instances(req.by_type).await?;
    Ok(ListInstancesResponse {
        instances: instances.into_iter().map(Into::into).collect(),
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteInstanceDataRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteInstanceDataResponse {
    pub success: bool,
}

pub async fn delete_data(
    context: &HandlerContext,
    req: DeleteInstanceDataRequest,
) -> Result<DeleteInstanceDataResponse, anyhow::Error> {
    let instance = context
        .process_manager()
        .get_instance_by_name(req.name)
        .await?
        .ok_or_else(|| {
            JsonRpcError::new(
                JsonRpcErrorReason::ApplicationError(404),
                "Instance not found".to_string(),
                serde_json::Value::Null,
            )
        })?;

    context.process_manager().stop_instance(instance.id).await?;
    context.process_manager().delete_instance_data(instance.id).await?;

    Ok(DeleteInstanceDataResponse { success: true })
}
