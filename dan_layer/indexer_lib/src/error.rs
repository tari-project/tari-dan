//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::quorum_certificate_validations::QuorumCertificateValidationError;
use tari_engine_types::substate::SubstateId;
use tari_epoch_manager::EpochManagerError;

use crate::substate_cache::SubstateCacheError;

#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Could not get substate from {num_requested} validator nodes")]
    AllRequestsFailed { num_requested: usize },
    #[error("Validator node client error: {0}")]
    ValidatorNodeClientError(String),
    #[error("Invalid substate state")]
    InvalidSubstateState,
    #[error("Invalid substate value")]
    InvalidSubstateValue,
    #[error("Not found transaction for component address {0} and version {1}")]
    NotFoundTransaction(SubstateId, u32),
    #[error("Failed to get consensus constants: {0}")]
    FailedToGetCommitteeSize(String),
    #[error("Failed to parse transaction hash: {0}")]
    FailedToParseTransactionHash(String),
    #[error("Substate cache operation failed: {0}")]
    SubstateCacheError(#[from] SubstateCacheError),
    #[error("Quorum certificate validation error: {0}")]
    QuorumCertificateValidationError(#[from] QuorumCertificateValidationError),
    #[error("Missing quorum certificate")]
    MissingQuorumCertificate,
}
