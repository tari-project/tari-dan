// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use log::*;
use minotari_app_grpc::tari_rpc::{
    self as grpc,
    ConsensusConstants,
    GetActiveValidatorNodesResponse,
    RegisterValidatorNodeResponse,
};
use tari_shutdown::ShutdownSignal;
use tokio::sync::{mpsc, oneshot};

use crate::{
    config::{Channels, Config, ExecutableConfig},
    constants::DEFAULT_VALIDATOR_NODE_BINARY_PATH,
    minotari::{Minotari, TipStatus},
    monitoring::{process_status_alert, process_status_log, ProcessStatus, Transaction},
    process::Process,
};

pub struct ProcessManager {
    pub base_dir: PathBuf,
    pub validator_base_dir: PathBuf,
    pub validator_config: ExecutableConfig,
    pub wallet_config: ExecutableConfig,
    pub process: Process,
    pub shutdown_signal: ShutdownSignal,
    pub rx_request: mpsc::Receiver<ManagerRequest>,
    pub chain: Minotari,
    pub alerting_config: Channels,
}

impl ProcessManager {
    pub fn new(config: Config, shutdown_signal: ShutdownSignal) -> (Self, ManagerHandle) {
        let (tx_request, rx_request) = mpsc::channel(1);
        let this = Self {
            base_dir: config.base_dir.clone(),
            validator_base_dir: config.vn_base_dir,
            validator_config: config.executable_config[0].clone(),
            wallet_config: config.executable_config[1].clone(),
            process: Process::new(),
            shutdown_signal,
            rx_request,
            chain: Minotari::new(
                config.base_node_grpc_address,
                config.base_wallet_grpc_address,
                config.vn_registration_file,
            ),
            alerting_config: config.channel_config,
        };
        (this, ManagerHandle::new(tx_request))
    }

    pub async fn start(mut self) -> anyhow::Result<()> {
        info!("Starting validator node process");

        // clean_stale_pid_file(self.base_dir.clone().join(DEFAULT_VALIDATOR_PID_PATH)).await?;

        let vn_binary_path = self
            .validator_config
            .clone()
            .executable_path
            .unwrap_or(PathBuf::from(DEFAULT_VALIDATOR_NODE_BINARY_PATH));

        let vn_base_dir = self.base_dir.join(self.validator_base_dir);

        // get child channel to communicate with the validator node process
        let cc = self
            .process
            .start_validator(vn_binary_path, vn_base_dir, self.base_dir, self.alerting_config)
            .await;
        if cc.is_none() {
            todo!("Create new validator node process event listener for fetched existing PID from OS");
        }
        let cc = cc.unwrap();

        // spawn logging and alerting tasks to process status updates
        tokio::spawn(async move {
            process_status_log(cc.rx_log).await;
        });
        tokio::spawn(async move {
            process_status_alert(cc.rx_alert, cc.cfg_alert).await;
        });

        self.chain.bootstrap().await?;

        let mut last_registered_at_block = 0;
        info!("Setup completed: connected to base node and wallet, ready to receive requests");
        loop {
            tokio::select! {
                Some(req) = self.rx_request.recv() => {
                    match req {
                        ManagerRequest::GetTipInfo { reply } => {
                            let response = self.chain.get_tip_status().await?;

                            // send latest block height to logging
                            if let Err(e) = cc.tx_log.send(ProcessStatus::WarnExpiration(response.height(), last_registered_at_block)).await {
                                error!("Failed to send tip status update to monitoring: {}", e);
                            }
                            // send latest block height to alerting
                            if let Err(e) = cc.tx_alert.send(ProcessStatus::WarnExpiration(response.height(), last_registered_at_block)).await {
                                error!("Failed to send tip status update to alerting: {}", e);
                            }

                            drop(reply.send(Ok(response)));
                        }
                        ManagerRequest::GetActiveValidatorNodes { reply } => {
                            let response = self.chain.get_active_validator_nodes().await?;
                            drop(reply.send(Ok(response)));
                        }
                        ManagerRequest::RegisterValidatorNode { block, reply } => {
                            let response = self.chain.register_validator_node().await?;
                            last_registered_at_block = block;

                            // send registration response to logger
                            if let Err(e) = cc.tx_log.send(ProcessStatus::Submitted(Transaction::new(response.clone(), block))).await {
                                error!("Failed to send node registration update to monitoring: {}", e);
                            }
                            // send registration response to alerting
                            if let Err(e) = cc.tx_alert.send(ProcessStatus::Submitted(Transaction::new(response.clone(), block))).await {
                                error!("Failed to send node registration update to alerting: {}", e);
                            }

                            drop(reply.send(Ok(response)));
                        },
                        ManagerRequest::GetConsensusConstants { block, reply } => {
                            let response = self.chain.get_consensus_constants(block).await?;
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
        block: u64,
        reply: Reply<grpc::ConsensusConstants>,
    },
    RegisterValidatorNode {
        block: u64,
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

    pub async fn get_consensus_constants(&mut self, block: u64) -> anyhow::Result<ConsensusConstants> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(ManagerRequest::GetConsensusConstants { block, reply: tx })
            .await?;
        rx.await?
    }

    pub async fn register_validator_node(&mut self, block: u64) -> anyhow::Result<RegisterValidatorNodeResponse> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(ManagerRequest::RegisterValidatorNode { block, reply: tx })
            .await?;
        rx.await?
    }

    pub async fn get_tip_info(&mut self) -> anyhow::Result<TipStatus> {
        let (tx, rx) = oneshot::channel();
        self.tx_request.send(ManagerRequest::GetTipInfo { reply: tx }).await?;
        rx.await?
    }
}
