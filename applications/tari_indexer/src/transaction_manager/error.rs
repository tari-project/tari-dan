//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::IsNotFoundError;
use tari_epoch_manager::EpochManagerError;
use tari_indexer_lib::{error::IndexerError, transaction_autofiller::TransactionAutofillerError};

#[derive(Debug, thiserror::Error)]
pub enum TransactionManagerError {
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Rpc call failed for all ({committee_size}) validators: {}", .last_error.as_deref().unwrap_or("unknown"))]
    AllValidatorsFailed {
        committee_size: usize,
        last_error: Option<String>,
    },
    #[error("No committee at present. Try again later")]
    NoCommitteeMembers,
    #[error("{entity} not found: {key}")]
    NotFound { entity: &'static str, key: String },
    #[error(transparent)]
    SubstateScanningError(#[from] IndexerError),
    #[error(transparent)]
    TransactionAutofillerError(#[from] TransactionAutofillerError),
    #[error(transparent)]
    UnexpectedError(#[from] anyhow::Error),
}

impl IsNotFoundError for TransactionManagerError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::NotFound { .. })
    }
}
