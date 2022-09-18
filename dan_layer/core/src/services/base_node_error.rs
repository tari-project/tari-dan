use tari_common_types::types::FixedHashSizeError;
use thiserror::Error;

#[derive(Error, Debug)]

pub enum BaseNodeError {
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
