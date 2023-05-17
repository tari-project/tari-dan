//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

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
}
