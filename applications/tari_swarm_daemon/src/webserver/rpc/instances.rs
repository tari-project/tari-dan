//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};
use serde::{Deserialize, Serialize};

use crate::webserver::context::HandlerContext;

pub type StartInstanceRequest = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
