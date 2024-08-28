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

#[derive(Debug, Clone, Deserialize)]
pub struct MinotariWalletBurnFundsRequest {
    pub amount: u64,
    pub wallet_instance_id: InstanceId,
    pub account_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MinotariWalletBurnFundsResponse {
    pub url: String,
}

pub async fn burn_funds(
    context: &HandlerContext,
    req: MinotariWalletBurnFundsRequest,
) -> Result<MinotariWalletBurnFundsResponse, anyhow::Error> {
    let misc_path = context.config().base_dir.join("misc");
    let file_name = context
        .process_manager()
        .burn_funds(req.amount, req.wallet_instance_id, req.account_name, misc_path)
        .await?;

    Ok(MinotariWalletBurnFundsResponse {
        // Panic: filename is always valid utf-8
        url: format!("/misc/{}", file_name.to_str().expect("Invalid file name")),
    })
}
