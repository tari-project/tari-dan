//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::IsNotFoundError;
use tari_indexer_lib::error::IndexerError;

#[derive(Debug, thiserror::Error)]
pub enum TransactionManagerError {
    #[error("Committee provider error: {0}")]
    CommitteeProviderError(String),
    #[error("Rpc call failed for all ({committee_size}) validators")]
    AllValidatorsFailed { committee_size: usize },
    #[error("No committee at present. Try again later")]
    NoCommitteeMembers,
    #[error("{entity} not found: {key}")]
    NotFound { entity: &'static str, key: String },
    #[error(transparent)]
    SubstateScanningError(#[from] IndexerError),
    #[error(transparent)]
    UnexpectedError(#[from] anyhow::Error),
}

impl IsNotFoundError for TransactionManagerError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::NotFound { .. })
    }
}
