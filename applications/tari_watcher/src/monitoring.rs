// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::*;
use minotari_app_grpc::tari_rpc::RegisterValidatorNodeResponse;
use tokio::{process::Child, sync::mpsc, time::sleep};

use crate::{
    alerting::{Alerting, MatterMostNotifier},
    config::Channels,
    constants::{
        CONSENSUS_CONSTANT_REGISTRATION_DURATION,
        DEFAULT_PROCESS_MONITORING_INTERVAL,
        DEFAULT_THRESHOLD_WARN_EXPIRATION,
    },
};

#[derive(Debug)]
pub struct Transaction {
    id: u64,
    block: u64,
}

impl Transaction {
    pub fn new(response: RegisterValidatorNodeResponse, block: u64) -> Self {
        Self {
            id: response.transaction_id,
            block,
        }
    }
}

#[derive(Debug)]
pub enum ProcessStatus {
    Running,
    Exited(i32), // status code
    Crashed,
    InternalError(String),
    Submitted(Transaction),
    WarnExpiration(u64, u64), // current block and last registered block
}

pub async fn monitor_child(
    mut child: Child,
    tx_logging: mpsc::Sender<ProcessStatus>,
    tx_alerting: mpsc::Sender<ProcessStatus>,
) {
    loop {
        sleep(DEFAULT_PROCESS_MONITORING_INTERVAL).await;

        // if the child process encountered an unexpected error, not related to the process itself
        if child.try_wait().is_err() {
            let err = child.try_wait().err().unwrap();
            let err_msg = err.to_string();
            tx_logging
                .send(ProcessStatus::InternalError(err_msg.clone()))
                .await
                .expect("Failed to send internal error status to logging");
            tx_alerting
                .send(ProcessStatus::InternalError(err_msg))
                .await
                .expect("Failed to send internal error status to alerting");
            break;
        }
        // process has finished, intentional or not, if it has some status
        if let Some(status) = child.try_wait().expect("Failed to poll child process") {
            if !status.success() {
                tx_logging
                    .send(ProcessStatus::Crashed)
                    .await
                    .expect("Failed to send status to logging");
                tx_alerting
                    .send(ProcessStatus::Crashed)
                    .await
                    .expect("Failed to send status to alerting");
                break;
            }
            tx_logging
                .send(ProcessStatus::Exited(status.code().unwrap_or(0)))
                .await
                .expect("Failed to send process exit status to logging");
            tx_alerting
                .send(ProcessStatus::Exited(status.code().unwrap_or(0)))
                .await
                .expect("Failed to send process exit status to alerting");
            break;
        }
        // process is still running
        tx_logging
            .send(ProcessStatus::Running)
            .await
            .expect("Failed to send process running status to logging");
        tx_alerting
            .send(ProcessStatus::Running)
            .await
            .expect("Failed to send process running status to alerting");
    }
}

pub fn is_registration_near_expiration(curr_block: u64, last_registered_block: u64) -> bool {
    last_registered_block != 0 &&
        curr_block + DEFAULT_THRESHOLD_WARN_EXPIRATION >=
            last_registered_block + CONSENSUS_CONSTANT_REGISTRATION_DURATION
}

pub async fn process_status_log(mut rx: mpsc::Receiver<ProcessStatus>) {
    while let Some(status) = rx.recv().await {
        match status {
            ProcessStatus::Exited(code) => {
                error!("Validator node process exited with code {}", code);
                break;
            },
            ProcessStatus::InternalError(err) => {
                error!("Validator node process exited with error: {}", err);
                break;
            },
            ProcessStatus::Crashed => {
                error!("Validator node process crashed");
                break;
            },
            ProcessStatus::Running => {
                // all good, process is still running
            },
            ProcessStatus::Submitted(tx) => {
                info!(
                    "Validator node registration submitted (tx: {}, block: {})",
                    tx.id, tx.block
                );
            },
            ProcessStatus::WarnExpiration(block, last_reg_block) => {
                if is_registration_near_expiration(block, last_reg_block) {
                    let expiration_block = last_reg_block + CONSENSUS_CONSTANT_REGISTRATION_DURATION;
                    warn!(
                        "Validator node registration expires at block {}, current block: {}",
                        expiration_block, block
                    );
                }
            },
        }
    }
}

pub async fn process_status_alert(mut rx: mpsc::Receiver<ProcessStatus>, cfg: Channels) {
    let mut mattermost: Option<MatterMostNotifier> = None;
    if cfg.mattermost.enabled {
        let cfg = cfg.mattermost.clone();
        info!("MatterMost alerting enabled");
        mattermost = Some(MatterMostNotifier::new(cfg.server_url, cfg.channel_id, cfg.credentials));
    } else {
        info!("MatterMost alerting disabled");
    }

    while let Some(status) = rx.recv().await {
        match status {
            ProcessStatus::Exited(code) => {
                if let Some(mm) = &mut mattermost {
                    mm.alert(&format!("Validator node process exited with code {}", code))
                        .await
                        .expect("Failed to send alert to MatterMost");
                }
            },
            ProcessStatus::InternalError(err) => {
                if let Some(mm) = &mut mattermost {
                    mm.alert(&format!("Validator node process internal error: {}", err))
                        .await
                        .expect("Failed to send alert to MatterMost");
                }
            },
            ProcessStatus::Crashed => {
                if let Some(mm) = &mut mattermost {
                    mm.alert("Validator node process crashed")
                        .await
                        .expect("Failed to send alert to MatterMost");
                }
            },
            ProcessStatus::Running => {
                // all good, process is still running, send heartbeat to channel(s)
                if let Some(mm) = &mut mattermost {
                    if mm.ping().await.is_err() {
                        warn!("Failed to send heartbeat to MatterMost");
                    }
                }
            },
            ProcessStatus::Submitted(tx) => {
                if let Some(mm) = &mut mattermost {
                    mm.alert(&format!(
                        "Validator node registration submitted (tx: {}, block: {})",
                        tx.id, tx.block
                    ))
                    .await
                    .expect("Failed to send alert to MatterMost");
                }
            },
            ProcessStatus::WarnExpiration(block, last_reg_block) => {
                if is_registration_near_expiration(block, last_reg_block) {
                    if let Some(mm) = &mut mattermost {
                        let expiration_block = last_reg_block + CONSENSUS_CONSTANT_REGISTRATION_DURATION;
                        mm.alert(&format!(
                            "Validator node registration expires at block {}, current block: {}",
                            expiration_block, block,
                        ))
                        .await
                        .expect("Failed to send alert to MatterMost");
                    }
                }
            },
        }
    }
}
