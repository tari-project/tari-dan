//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_bor::BorError;
use tari_networking::{NetworkingError, PeerId};
use tari_ootle_common_types::optional::IsNotFoundError;
use tari_rpc_framework::{RpcError, RpcStatus};

#[derive(Debug, thiserror::Error)]
pub enum ValidatorNodeRpcClientError {
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
    #[error("Timed out after {timeout:.0?} connecting to peer {peer_id}")]
    ConnectTimeout { peer_id: PeerId, timeout: Duration },
    #[error("Node did not finish its response within {timeout:.0?}")]
    ResponseTimeout { timeout: Duration },
}

impl ValidatorNodeRpcClientError {
    pub fn status(&self) -> Option<&RpcStatus> {
        match self {
            Self::RpcStatusError(status) => Some(status),
            Self::RpcError(RpcError::RequestFailed(status)) => Some(status),
            _ => None,
        }
    }
}

impl IsNotFoundError for ValidatorNodeRpcClientError {
    fn is_not_found_error(&self) -> bool {
        match self {
            Self::RpcStatusError(status) => status.is_not_found(),
            Self::RpcError(RpcError::RequestFailed(status)) => status.is_not_found(),
            _ => false,
        }
    }
}
