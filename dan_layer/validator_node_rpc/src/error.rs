//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_comms::{
    connectivity::ConnectivityError,
    protocol::rpc::{RpcError, RpcStatus},
    types::CommsPublicKey,
};
use tari_dan_common_types::optional::IsNotFoundError;

#[derive(Debug, thiserror::Error)]
pub enum ValidatorNodeRpcClientError {
    #[error("Protocol violations for peer {peer}: {details}")]
    ProtocolViolation { peer: CommsPublicKey, details: String },
    #[error("Connectivity error:{0}")]
    ConnectivityError(#[from] ConnectivityError),
    #[error("RpcError: {0}")]
    RpcError(#[from] RpcError),
    #[error("Remote node returned error: {0}")]
    RpcStatusError(#[from] RpcStatus),
    #[error("Node sent invalid response: {0}")]
    InvalidResponse(anyhow::Error),
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
