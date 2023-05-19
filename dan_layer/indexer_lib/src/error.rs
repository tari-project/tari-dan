//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::substate::SubstateAddress;

#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error("Committee provider error: {0}")]
    CommitteeProviderError(String),
    #[error("Validator node client error: {0}")]
    ValidatorNodeClientError(String),
    #[error("Invalid substate state")]
    InvalidSubstateState,
    #[error("Invalid substate value")]
    InvalidSubstateValue,
    #[error("Not found transaction for component address {0} and version {1}")]
    NotFoundTransaction(SubstateAddress, u32),
    #[error("Failed to get consensus constants: {0}")]
    FailedToGetCommitteeSize(String),
    #[error("Failed to parse transaction hash: {0}")]
    FailedToParseTransactionHash(String),
}
