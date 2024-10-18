//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use log::warn;
use serde::{Deserialize, Serialize};
use tari_shutdown::Shutdown;
use tokio::select;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartMiningRequest {
    interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartMiningResponse {}

pub async fn start_mining(
    context: &HandlerContext,
    req: StartMiningRequest,
) -> Result<StartMiningResponse, anyhow::Error> {
    let shutdown = Shutdown::new();
    context.start_mining(shutdown.clone()).await?;

    let process_manager = context.process_manager().clone();
    tokio::spawn(async move {
        let shutdown_signal = shutdown.to_signal();
        let interval = tokio::time::interval(Duration::from_secs(req.interval_seconds));
        tokio::pin!(shutdown_signal);
        tokio::pin!(interval);
        loop {
            select! {
                _ = &mut shutdown_signal => {
                    break;
                }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsMiningRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsMiningResponse {
    result: bool,
}

pub async fn is_mining(context: &HandlerContext, _req: IsMiningRequest) -> Result<IsMiningResponse, anyhow::Error> {
    Ok(IsMiningResponse {
        result: context.is_mining().await,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopMiningRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopMiningResponse {
    result: bool,
}

pub async fn stop_mining(
    context: &HandlerContext,
    _req: StopMiningRequest,
) -> Result<StopMiningResponse, anyhow::Error> {
    context.stop_mining().await;
    Ok(StopMiningResponse { result: true })
}
