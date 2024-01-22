//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::BorError;
use tari_dan_common_types::{optional::IsNotFoundError, PeerAddress};
use tari_networking::NetworkingError;
use tari_rpc_framework::{RpcError, RpcStatus};

#[derive(Debug, thiserror::Error)]
pub enum ValidatorNodeRpcClientError {
    #[error("Protocol violations for peer {peer}: {details}")]
    ProtocolViolation { peer: PeerAddress, details: String },
    #[error("NetworkingError: {0}")]
    NetworkingError(#[from] NetworkingError),
    #[error("RpcError: {0}")]
    RpcError(#[from] RpcError),
    #[error("Remote node returned error: {0}")]
    RpcStatusError(#[from] RpcStatus),
    #[error("Node sent invalid response: {0}")]
    InvalidResponse(anyhow::Error),
    #[error("BorError: {0}")]
    BorError(#[from] BorError),
}

impl IsNotFoundError for ValidatorNodeRpcClientError {
    fn is_not_found_error(&self) -> bool {
        match self {
            ValidatorNodeRpcClientError::RpcStatusError(status) => status.is_not_found(),
            ValidatorNodeRpcClientError::RpcError(RpcError::RequestFailed(status)) => status.is_not_found(),
            _ => false,
        }
    }
}
