//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::error::Error as StdError;

use reqwest::{StatusCode, Url};
use tari_ootle_common_types::optional::IsNotFoundError;

/// reqwest hides the actual transport failure (timeout, connection refused, DNS error, ...) in its
/// error source chain — its own `Display` only says "error sending request for url (...)". This walks
/// the chain so the real cause, and any HTTP status, ends up in the message.
fn describe_request_error(err: &reqwest::Error) -> String {
    let mut msg = err.to_string();
    if let Some(status) = err.status() {
        msg = format!("{msg} (HTTP {})", status.as_u16());
    }
    let mut source = err.source();
    while let Some(cause) = source {
        msg = format!("{msg}: {cause}");
        source = cause.source();
    }
    msg
}

#[derive(Debug, thiserror::Error)]
pub enum IndexerJrpcClientError {
    #[error("Failed to deserialize response for method {method}: {source} - response: {response}")]
    DeserializeResponse {
        source: serde_json::Error,
        method: &'static str,
        response: serde_json::Value,
    },
    #[error("Failed to serialize request for method {method}: {source}")]
    SerializeRequest { method: String, source: serde_json::Error },
    #[error("Failed to send request: {}", describe_request_error(.source))]
    RequestFailed {
        #[from]
        source: reqwest::Error,
    },
    #[error("Request failed: code: {code} message: {message}")]
    RequestFailedWithStatus { code: i64, message: String },
    #[error("Invalid response: {message}")]
    InvalidResponse { message: String },
}

impl IsNotFoundError for IndexerJrpcClientError {
    fn is_not_found_error(&self) -> bool {
        match self {
            Self::RequestFailedWithStatus { code, .. } => *code == 404,
            Self::RequestFailed { source } => source.status().map(|s| s == StatusCode::NOT_FOUND).unwrap_or(false),
            _ => false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IndexerRestClientError {
    #[error("Failed to deserialize response for path {path}: {source}")]
    DeserializeResponse { source: anyhow::Error, path: String },
    #[error("Failed to serialize request for path {path}: {source}")]
    SerializeRequest { path: String, source: anyhow::Error },
    #[error("Failed to send request: {}", describe_request_error(.source))]
    RequestFailed {
        #[from]
        source: reqwest::Error,
    },
    #[error("Server returned an error: {} details: {}", describe_request_error(.source), .details.as_deref().unwrap_or("none"))]
    ErrorResponse {
        source: reqwest::Error,
        details: Option<String>,
    },
    /// The indexer answered with a 1xx or 3xx status, e.g. a redirect that was not followed. Such a response does not
    /// carry the requested resource.
    #[error("Unexpected HTTP status {status} from {url}")]
    UnexpectedStatus { status: StatusCode, url: Url },
    #[error("Request failed: code: {code} message: {message}")]
    RequestFailedWithStatus { code: i64, message: String },
    #[error("Invalid response: {message}")]
    InvalidResponse { message: String },
    #[error("Unable to construct request due to unsatisfied invariant: {details}")]
    RequestInvariant { details: String },
}

impl IsNotFoundError for IndexerRestClientError {
    fn is_not_found_error(&self) -> bool {
        match self {
            Self::RequestFailedWithStatus { code, .. } => *code == 404,
            Self::RequestFailed { source, .. } | Self::ErrorResponse { source, .. } => {
                source.status().map(|s| s == StatusCode::NOT_FOUND).unwrap_or(false)
            },
            _ => false,
        }
    }
}
