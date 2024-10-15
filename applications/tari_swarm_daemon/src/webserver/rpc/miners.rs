//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::webserver::context::HandlerContext;
use log::warn;
use nix::libc::select;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tari_shutdown::Shutdown;
use tokio::select;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartMiningRequest {
    interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartMiningResponse {}

pub async fn start_mining(context: &HandlerContext, req: StartMiningRequest) -> Result<StartMiningResponse, anyhow::Error> {
    let shutdown = Shutdown::new();
    let process_manager = context.process_manager().clone();
    tokio::spawn(async move {
        let interval = tokio::time::interval(Duration::from_secs(req.interval_seconds));
        tokio::pin!(interval);
        loop {
            select! {
                _ = interval.tick() => {
                    if let Err(error) = process_manager.mine_blocks(1).await {
                        warn!("Failed to mine a block: {error:?}");
                    }
                }
            }
        }
    });
    Ok(StartMiningResponse {})
}
