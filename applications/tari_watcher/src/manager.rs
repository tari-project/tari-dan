// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::*;
use minotari_app_grpc::tari_rpc::{
    self as grpc, ConsensusConstants, GetActiveValidatorNodesResponse, RegisterValidatorNodeResponse,
};
use tari_shutdown::ShutdownSignal;
use tokio::sync::{mpsc, oneshot};

use crate::{
    config::{Config, ExecutableConfig},
    forker::Forker,
    minotari::{Minotari, TipStatus},
};

pub struct ProcessManager {
    pub validator_config: ExecutableConfig,
    pub wallet_config: ExecutableConfig,
    pub forker: Forker,
    pub shutdown_signal: ShutdownSignal,
    pub rx_request: mpsc::Receiver<ManagerRequest>,
    pub chain: Minotari,
}

impl ProcessManager {
    pub fn new(config: Config, shutdown_signal: ShutdownSignal) -> (Self, ManagerHandle) {
        let (tx_request, rx_request) = mpsc::channel(1);
        let this = Self {
            validator_config: config.executable_config[0].clone(),
            wallet_config: config.executable_config[1].clone(),
            forker: Forker::new(),
            shutdown_signal,
            rx_request,
            chain: Minotari::new(
                config.base_node_grpc_address,
                config.base_wallet_grpc_address,
                config.vn_registration_file,
            ),
        };
        (this, ManagerHandle::new(tx_request))
    }

    pub async fn start(mut self) -> anyhow::Result<()> {
        info!("Starting validator node process");

        self.forker.start_validator(self.validator_config.clone()).await?;
        self.chain.bootstrap().await?;
        info!("Watcher process bootstrapped and connected to base node and wallet");

        loop {
            tokio::select! {
                Some(req) = self.rx_request.recv() => {
                    match req {
                        ManagerRequest::GetTipInfo { reply } => {
                            let response = self.chain.get_tip_status().await?;
                            drop(reply.send(Ok(response)));
                        }
                        ManagerRequest::GetActiveValidatorNodes { reply } => {
                            let response = self.chain.get_active_validator_nodes().await?;
                            drop(reply.send(Ok(response)));
                        }
                        ManagerRequest::RegisterValidatorNode { reply } => {
                            let response = self.chain.register_validator_node().await?;
                            drop(reply.send(Ok(response)));
                        },
                        ManagerRequest::GetConsensusConstants { reply, block_height } => {
                            let response = self.chain.get_consensus_constants(block_height).await?;
                            drop(reply.send(Ok(response)));
                        }
                    }
                }

                _ = self.shutdown_signal.wait() => {
                    info!("Shutting down process manager");
                    break;
                }
            }
        }

        Ok(())
    }
}

type Reply<T> = oneshot::Sender<anyhow::Result<T>>;

pub enum ManagerRequest {
    GetTipInfo {
        reply: Reply<TipStatus>,
    },
    GetActiveValidatorNodes {
        reply: Reply<Vec<GetActiveValidatorNodesResponse>>,
    },
    GetConsensusConstants {
        block_height: u64,
        reply: Reply<grpc::ConsensusConstants>,
    },
    RegisterValidatorNode {
        reply: Reply<RegisterValidatorNodeResponse>,
    },
}

pub struct ManagerHandle {
    tx_request: mpsc::Sender<ManagerRequest>,
}

impl ManagerHandle {
    pub fn new(tx_request: mpsc::Sender<ManagerRequest>) -> Self {
        Self { tx_request }
    }

    pub async fn get_active_validator_nodes(&mut self) -> anyhow::Result<Vec<GetActiveValidatorNodesResponse>> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(ManagerRequest::GetActiveValidatorNodes { reply: tx })
            .await?;
        rx.await?
    }

    pub async fn get_consensus_constants(&mut self, block_height: u64) -> anyhow::Result<ConsensusConstants> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(ManagerRequest::GetConsensusConstants {
                block_height,
                reply: tx,
            })
            .await?;
        rx.await?
    }

    pub async fn register_validator_node(&mut self) -> anyhow::Result<RegisterValidatorNodeResponse> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(ManagerRequest::RegisterValidatorNode { reply: tx })
            .await?;
        rx.await?
    }

    pub async fn get_tip_info(&mut self) -> anyhow::Result<TipStatus> {
        let (tx, rx) = oneshot::channel();
        self.tx_request.send(ManagerRequest::GetTipInfo { reply: tx }).await?;
        rx.await?
    }
}
