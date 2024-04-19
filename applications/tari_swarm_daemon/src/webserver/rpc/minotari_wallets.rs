//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{config::InstanceType, process_manager::InstanceId, webserver::context::HandlerContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinotariWalletCreateRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinotariWalletCreateResponse {
    pub instance_id: InstanceId,
}

pub async fn create(
    context: &HandlerContext,
    req: MinotariWalletCreateRequest,
) -> Result<MinotariWalletCreateResponse, anyhow::Error> {
    let instance_id = context
        .process_manager()
        .create_instance(req.name, InstanceType::MinoTariConsoleWallet, HashMap::new())
        .await?;

    Ok(MinotariWalletCreateResponse { instance_id })
}
