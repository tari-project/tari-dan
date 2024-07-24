//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_dan_storage::StorageError;
use tari_epoch_manager::EpochManagerError;
use tari_networking::NetworkingError;
use tokio::sync::{mpsc, oneshot};

use crate::{
    dry_run_transaction_processor::DryRunTransactionProcessorError,
    p2p::services::mempool::MempoolRequest,
    transaction_validators::TransactionValidationError,
};

#[derive(thiserror::Error, Debug)]
pub enum MempoolError {
    #[error("Invalid message: {0}")]
    InvalidMessage(#[from] anyhow::Error),
    #[error("Epoch Manager Error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Internal service request cancelled")]
    RequestCancelled,
    #[error("Consensus channel closed")]
    ConsensusChannelClosed,
    #[error("DryRunTransactionProcessor Error: {0}")]
    DryRunTransactionProcessorError(#[from] DryRunTransactionProcessorError),
    #[error("Storage Error: {0}")]
    StorageError(#[from] StorageError),
    #[error("Transaction validation error: {0}")]
    TransactionValidationError(#[from] TransactionValidationError),

    #[error("Network error: {0}")]
    NetworkingError(#[from] NetworkingError),
}

impl From<mpsc::error::SendError<MempoolRequest>> for MempoolError {
    fn from(_: mpsc::error::SendError<MempoolRequest>) -> Self {
        Self::RequestCancelled
    }
}

impl From<oneshot::error::RecvError> for MempoolError {
    fn from(_: oneshot::error::RecvError) -> Self {
        Self::RequestCancelled
    }
}
