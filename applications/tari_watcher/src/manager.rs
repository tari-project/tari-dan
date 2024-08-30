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
use tari_shutdown::{Shutdown, ShutdownSignal};
use tokio::{
    sync::{
        mpsc::{self, Receiver},
        oneshot,
    },
    task::JoinHandle,
};

use crate::{
    config::{Channels, Config, ExecutableConfig},
    constants::DEFAULT_VALIDATOR_NODE_BINARY_PATH,
    minotari::{Minotari, TipStatus},
    monitoring::{process_status_alert, process_status_log, ProcessStatus, Transaction},
    process::{start_validator, ChildChannel},
};

pub struct ProcessManager {
    pub base_dir: PathBuf,
    pub validator_base_dir: PathBuf,
    pub validator_config: ExecutableConfig,
    pub wallet_config: ExecutableConfig,
    pub shutdown_signal: ShutdownSignal, // listen for keyboard exit signal
    pub trigger_signal: Shutdown,        // triggered when validator auto-restart is disabled
    pub rx_request: mpsc::Receiver<ManagerRequest>,
    pub chain: Minotari,
    pub alerting_config: Channels,
    pub auto_restart: bool,
}

pub struct ChannelReceivers {
    pub rx_log: Receiver<ProcessStatus>,
    pub rx_alert: Receiver<ProcessStatus>,
    pub cfg_alert: Channels,
    pub task: JoinHandle<()>,
}

impl ProcessManager {
    pub fn new(config: Config, shutdown_signal: ShutdownSignal, trigger_signal: Shutdown) -> (Self, ManagerHandle) {
        let (tx_request, rx_request) = mpsc::channel(1);
        let this = Self {
            base_dir: config.base_dir.clone(),
            validator_base_dir: config.vn_base_dir,
            validator_config: config.executable_config[0].clone(),
            wallet_config: config.executable_config[1].clone(),
            shutdown_signal,
            trigger_signal,
            rx_request,
            chain: Minotari::new(
                config.base_node_grpc_address,
                config.base_wallet_grpc_address,
                config.vn_registration_file,
            ),
            alerting_config: config.channel_config,
            auto_restart: config.auto_restart,
        };
        (this, ManagerHandle::new(tx_request))
    }

    pub async fn start_request_handler(mut self) -> anyhow::Result<ChannelReceivers> {
        info!("Starting validator node process");

        // clean_stale_pid_file(self.base_dir.clone().join(DEFAULT_VALIDATOR_PID_PATH)).await?;

        self.chain.bootstrap().await?;

        let cc = self.start_child_process().await;

        let mut last_registered_at_block = 0;
        info!("Setup completed: connected to base node and wallet, ready to receive requests");
        let task_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(req) = self.rx_request.recv() => {
                        match req {
                            ManagerRequest::GetTipInfo { reply } => {
                                let response = match self.chain.get_tip_status().await {
                                    Ok(resp) => resp,
                                    Err(e) => {
                                        error!("Failed to get tip status: {}", e);
                                        continue;
                                    }
                                };

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
                                let response = match self.chain.get_active_validator_nodes().await {
                                    Ok(resp) => resp,
                                    Err(e) => {
                                        error!("Failed to get active validator nodes: {}", e);
                                        continue;
                                    }
                                };
                                drop(reply.send(Ok(response)));
                            }
                            ManagerRequest::RegisterValidatorNode { block, reply } => {
                                let response = match self.chain.register_validator_node().await {
                                    Ok(resp) => resp,
                                    Err(e) => {
                                        error!("Failed to register validator node: {}", e);
                                        continue;
                                    }
                                };
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
                                let response = match self.chain.get_consensus_constants(block).await {
                                    Ok(resp) => resp,
                                    Err(e) => {
                                        error!("Failed to get consensus constants: {}", e);
                                        continue;
                                    }
                                };
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
        });

        Ok(ChannelReceivers {
            rx_log: cc.rx_log,
            rx_alert: cc.rx_alert,
            cfg_alert: cc.cfg_alert,
            task: task_handle,
        })
    }

    async fn start_child_process(&self) -> ChildChannel {
        let vn_binary_path = self
            .validator_config
            .clone()
            .executable_path
            .unwrap_or(PathBuf::from(DEFAULT_VALIDATOR_NODE_BINARY_PATH));

        let vn_base_dir = self.base_dir.join(self.validator_base_dir.clone());

        // get child channel to communicate with the validator node process
        let cc = start_validator(
            vn_binary_path,
            vn_base_dir,
            self.base_dir.clone(),
            self.alerting_config.clone(),
            self.auto_restart,
            self.trigger_signal.clone(),
        )
        .await;
        if cc.is_none() {
            todo!("Create new validator node process event listener for fetched existing PID from OS");
        }

        cc.unwrap()
    }
}

pub async fn start_receivers(
    rx_log: mpsc::Receiver<ProcessStatus>,
    rx_alert: mpsc::Receiver<ProcessStatus>,
    cfg_alert: Channels,
    constants: ConsensusConstants,
) {
    let const_copy = constants.clone();
    // spawn logging and alerting tasks to process status updates
    tokio::spawn(async move {
        process_status_log(rx_log, const_copy).await;
        warn!("Logging task has exited");
    });
    tokio::spawn(async move {
        process_status_alert(rx_alert, cfg_alert, constants).await;
        warn!("Alerting task has exited");
    });
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

#[derive(Clone)]
pub struct ManagerHandle {
    pub tx_request: mpsc::Sender<ManagerRequest>,
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
