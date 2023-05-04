//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[derive(Debug, Clone, thiserror::Error)]
pub enum TransactionManagerError {
    #[error("Committee provider error: {0}")]
    CommitteeProviderError(String),
    #[error("Rpc call failed for all ({committee_size}) validators")]
    AllValidatorsFailed { committee_size: usize },
}
