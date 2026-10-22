//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::optional::IsNotFoundError;
use tari_ootle_wallet_sdk::{
    apis::{
        accounts::AccountsApiError,
        config::ConfigApiError,
        key_manager::KeyManagerApiError,
        stealth_outputs::StealthOutputsApiError,
        substate::SubstateApiError,
    },
    storage::WalletStorageError,
};

#[derive(Debug, thiserror::Error)]
pub enum StealthScannerApiError {
    #[error("Config error: {0}")]
    ConfigError(#[from] ConfigApiError),
    #[error("Network interface error: {0}")]
    NetworkInterfaceError(anyhow::Error),
    #[error("Unexpected response: {details}")]
    UnexpectedResponse { details: String },
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Stealth outputs error: {0}")]
    StealthOutputsError(#[from] StealthOutputsApiError),
    #[error("Key manager error: {0}")]
    KeyManagerError(#[from] KeyManagerApiError),
    #[error("Account API error: {0}")]
    AccountApiError(#[from] AccountsApiError),
    #[error("Substate API error: {0}")]
    SubstateApiError(#[from] SubstateApiError),
    #[error("The UTXO scanner has shut down")]
    ScannerShutdown,
    #[error("The UTXO scan did not finish within {timeout:.0?}")]
    ScanTimedOut { timeout: std::time::Duration },
}

impl IsNotFoundError for StealthScannerApiError {
    fn is_not_found_error(&self) -> bool {
        match self {
            Self::StoreError(e) => e.is_not_found_error(),
            Self::StealthOutputsError(e) => e.is_not_found_error(),
            _ => false,
        }
    }
}
