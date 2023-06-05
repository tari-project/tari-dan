//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::FixedHashSizeError;
use tari_dan_common_types::optional::IsNotFoundError;
use thiserror::Error;

#[derive(Error, Debug)]

pub enum BaseNodeClientError {
    #[error("Could not connect to base node")]
    ConnectionError,
    #[error("Connection error: {0}")]
    GrpcConnection(#[from] tonic::transport::Error),
    #[error("GRPC error: {0}")]
    GrpcStatus(#[from] tonic::Status),
    #[error("Peer sent an invalid message: {0}")]
    InvalidPeerMessage(String),
    #[error("Hash size error: {0}")]
    HashSizeError(#[from] FixedHashSizeError),
}

impl IsNotFoundError for BaseNodeClientError {
    fn is_not_found_error(&self) -> bool {
        if let Self::GrpcStatus(status) = self {
            status.code() == tonic::Code::NotFound
        } else {
            false
        }
    }
}
