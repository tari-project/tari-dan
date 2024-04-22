//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};

use crate::webserver::context::HandlerContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MineRequest {
    num_blocks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MineResponse {}

pub async fn mine(context: &HandlerContext, req: MineRequest) -> Result<MineResponse, anyhow::Error> {
    context.process_manager().mine_blocks(req.num_blocks).await?;
    Ok(MineResponse {})
}
