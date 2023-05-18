//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::ComponentAddress;

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
    NotFoundTransaction(ComponentAddress, u32),
    #[error("Failed to get consensus constants: {0}")]
    FailedToGetCommitteeSize(String),
}
