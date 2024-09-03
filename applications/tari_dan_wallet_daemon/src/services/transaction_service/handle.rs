//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::SubstateRequirement;
use tari_dan_wallet_sdk::models::NewAccountInfo;
use tari_engine_types::commit_result::ExecuteResult;
use tari_transaction::{Transaction, TransactionId};
use tokio::sync::{mpsc, oneshot};

use super::TransactionServiceError;
use crate::services::Reply;

#[derive(Debug)]
pub(super) enum TransactionServiceRequest {
    SubmitTransaction {
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
        new_account_info: Option<NewAccountInfo>,
        reply: Reply<Result<TransactionId, TransactionServiceError>>,
    },

    SubmitDryRunTransaction {
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
        reply: Reply<Result<ExecuteResult, TransactionServiceError>>,
    },
}

#[derive(Debug, Clone)]
pub struct TransactionServiceHandle {
    sender: mpsc::Sender<TransactionServiceRequest>,
}

impl TransactionServiceHandle {
    pub(super) fn new(sender: mpsc::Sender<TransactionServiceRequest>) -> Self {
        Self { sender }
    }
}

impl TransactionServiceHandle {
    pub async fn submit_transaction(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
    ) -> Result<TransactionId, TransactionServiceError> {
        self.submit_transaction_with_opts(transaction, required_substates, None)
            .await
    }

    pub async fn submit_transaction_with_new_account(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
        new_account_info: NewAccountInfo,
    ) -> Result<TransactionId, TransactionServiceError> {
        self.submit_transaction_with_opts(transaction, required_substates, Some(new_account_info))
            .await
    }

    pub async fn submit_dry_run_transaction(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
    ) -> Result<ExecuteResult, TransactionServiceError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(TransactionServiceRequest::SubmitDryRunTransaction {
                transaction,
                required_substates,
                reply: reply_tx,
            })
            .await
            .map_err(|_| TransactionServiceError::ServiceShutdown)?;
        reply_rx.await.map_err(|_| TransactionServiceError::ServiceShutdown)?
    }

    pub async fn submit_transaction_with_opts(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
        new_account_info: Option<NewAccountInfo>,
    ) -> Result<TransactionId, TransactionServiceError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(TransactionServiceRequest::SubmitTransaction {
                transaction,
                required_substates,
                new_account_info,
                reply: reply_tx,
            })
            .await
            .map_err(|_| TransactionServiceError::ServiceShutdown)?;
        reply_rx.await.map_err(|_| TransactionServiceError::ServiceShutdown)?
    }
}
