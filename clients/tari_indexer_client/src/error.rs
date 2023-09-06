//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use reqwest::StatusCode;
use tari_dan_common_types::optional::IsNotFoundError;

#[derive(Debug, thiserror::Error)]
pub enum IndexerClientError {
    #[error("Failed to deserialize response for method {method}: {source}")]
    DeserializeResponse { source: serde_json::Error, method: String },
    #[error("Failed to serialize request for method {method}: {source}")]
    SerializeRequest { method: String, source: serde_json::Error },
    #[error("Failed to send request: {source}")]
    RequestFailed {
        #[from]
        source: reqwest::Error,
    },
    #[error("Request failed: code: {code} message: {message}")]
    RequestFailedWithStatus { code: i64, message: String },
    #[error("Invalid response: {message}")]
    InvalidResponse { message: String },
}

impl IsNotFoundError for IndexerClientError {
    fn is_not_found_error(&self) -> bool {
        match self {
            Self::RequestFailedWithStatus { code, .. } => *code == 404,
            Self::RequestFailed { source } => source.status().map(|s| s == StatusCode::NOT_FOUND).unwrap_or(false),
            _ => false,
        }
    }
}
