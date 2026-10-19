//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_indexer_client::error::IndexerRestClientError;

use crate::{
    provider::{VerifyError, input_resolver::TransactionInputResolverError},
    wallet::WalletError,
};

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("Indexer client error: {0}")]
    IndexerClientError(#[from] IndexerRestClientError),
    #[error("Transaction encoding error: {source}")]
    TransactionEncodeError {
        #[from]
        source: tari_bor::BorError,
    },
    #[error("Transaction input resolution error: {error}")]
    TransactionInputResolutionError {
        #[from]
        error: TransactionInputResolverError,
    },
    #[error(transparent)]
    WalletError(WalletError),
    #[error(transparent)]
    Unverified(VerifyError),
    #[error("Unknown provider error: {0}")]
    Other(String),
}

impl<E: Into<WalletError>> From<E> for ProviderError {
    fn from(err: E) -> Self {
        ProviderError::WalletError(err.into())
    }
}

impl From<VerifyError> for ProviderError {
    fn from(err: VerifyError) -> Self {
        match err {
            VerifyError::IndexerClientError(err) => ProviderError::IndexerClientError(err),
            err => ProviderError::Unverified(err),
        }
    }
}

impl ProviderError {
    pub fn other<S: Into<String>>(msg: S) -> Self {
        ProviderError::Other(msg.into())
    }
}
