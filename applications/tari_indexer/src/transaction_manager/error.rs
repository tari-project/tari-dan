//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::IsNotFoundError;
use tari_epoch_manager::base_layer::EpochManagerError;

#[derive(Debug, thiserror::Error)]
pub enum TransactionManagerError {
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Rpc call failed for all ({committee_size}) validators")]
    AllValidatorsFailed { committee_size: usize },
    #[error("No committee at present. Try again later")]
    NoCommitteeMembers,
    #[error("{entity} not found: {key}")]
    NotFound { entity: &'static str, key: String },
}

impl IsNotFoundError for TransactionManagerError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::NotFound { .. })
    }
}
