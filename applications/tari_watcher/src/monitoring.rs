// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use anyhow::Error;
use log::*;
use minotari_app_grpc::tari_rpc::RegisterValidatorNodeResponse;
use tokio::{process::Child, sync::mpsc, time::sleep};

use crate::constants::{
    CONSENSUS_CONSTANT_REGISTRATION_DURATION,
    DEFAULT_PROCESS_MONITORING_INTERVAL,
    DEFAULT_THRESHOLD_WARN_EXPIRATION,
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
    InternalError(Error),
    Submitted(Transaction),
    WarnExpiration(u64),
}

pub async fn monitor_child(mut child: Child, tx: mpsc::Sender<ProcessStatus>) {
    loop {
        sleep(DEFAULT_PROCESS_MONITORING_INTERVAL).await;

        // if the child process encountered an unexpected error, not related to the process itself
        if child.try_wait().is_err() {
            let err = child.try_wait().err().unwrap();
            tx.send(ProcessStatus::InternalError(err.into()))
                .await
                .expect("Failed to send internal error status");
            break;
        }
        // process has finished, intentional or not, if it has some status
        if let Some(status) = child.try_wait().expect("Failed to poll child process") {
            if !status.success() {
                tx.send(ProcessStatus::Crashed).await.expect("Failed to send status");
                break;
            }
            tx.send(ProcessStatus::Exited(status.code().unwrap_or(0)))
                .await
                .expect("Failed to send process exit status");
            break;
        }
        // process is still running
        tx.send(ProcessStatus::Running)
            .await
            .expect("Failed to send process running status");
    }
}

pub async fn read_status(mut rx: mpsc::Receiver<ProcessStatus>) {
    let mut last_registered_at_block = 0;
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
                last_registered_at_block = tx.block;
            },
            ProcessStatus::WarnExpiration(block) => {
                if last_registered_at_block != 0 &&
                    block + DEFAULT_THRESHOLD_WARN_EXPIRATION >=
                        last_registered_at_block + CONSENSUS_CONSTANT_REGISTRATION_DURATION
                {
                    warn!(
                        "Validator node registration expires at block {} ({} blocks remaining)",
                        last_registered_at_block + CONSENSUS_CONSTANT_REGISTRATION_DURATION,
                        last_registered_at_block + CONSENSUS_CONSTANT_REGISTRATION_DURATION - block
                    );
                }
            },
        }
    }
}
