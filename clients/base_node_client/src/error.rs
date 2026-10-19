//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::optional::IsNotFoundError;
use tari_sidechain::FixedHashSizeError;
use thiserror::Error;

#[derive(Error, Debug)]

pub enum BaseNodeClientError {
    #[error("Could not connect to base node")]
    ConnectionError,
    #[error("Connection error: {0}")]
    GrpcConnection(#[from] tonic::transport::Error),
    #[error("GRPC error: {code} {message}")]
    GrpcStatus { code: tonic::Code, message: String },
    #[error("Peer sent an invalid message: {0}")]
    InvalidPeerMessage(String),
    #[error("Hash size error: {0}")]
    HashSizeError(#[from] FixedHashSizeError),
}

impl IsNotFoundError for BaseNodeClientError {
    fn is_not_found_error(&self) -> bool {
        if let Self::GrpcStatus { code, .. } = self {
            *code == tonic::Code::NotFound
        } else {
            false
        }
    }
}

impl From<tonic::Status> for BaseNodeClientError {
    fn from(status: tonic::Status) -> Self {
        // To limit the size of BaseNodeClientError (to keep clippy happy), we dont include the entire Status instance
        Self::GrpcStatus {
            code: status.code(),
            message: status.message().to_string(),
        }
    }
}
