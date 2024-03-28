//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_wallet_sdk::apis::transaction::TransactionApiError;

#[derive(Debug, thiserror::Error)]
pub enum TransactionServiceError {
    #[error("Service shutdown")]
    ServiceShutdown,
    #[error("Transaction API error: {0}")]
    TransactionApiError(#[from] TransactionApiError),
    #[error("Dry run transaction failed: {details}")]
    DryRunTransactionFailed { details: String },
}
