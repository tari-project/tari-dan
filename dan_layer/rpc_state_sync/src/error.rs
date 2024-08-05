//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::hotstuff::{HotStuffError, ProposalValidationError};
use tari_dan_storage::{
    consensus_models::{BlockId, TransactionPoolError},
    StorageError,
};
use tari_epoch_manager::EpochManagerError;
use tari_rpc_framework::{RpcError, RpcStatus};
use tari_state_tree::{Hash, JmtStorageError};
use tari_validator_node_rpc::ValidatorNodeRpcClientError;

#[derive(Debug, thiserror::Error)]
pub enum CommsRpcConsensusSyncError {
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("RPC error: {0}")]
    RpcError(#[from] RpcError),
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
    #[error("Validator node client error: {0}")]
    ValidatorNodeClientError(#[from] ValidatorNodeRpcClientError),
    #[error("Transaction pool error: {0}")]
    TransactionPoolError(#[from] TransactionPoolError),
    #[error("Invalid response: {0}")]
    InvalidResponse(anyhow::Error),
    #[error("Block {block_id} failed SafeNode predicate")]
    BlockNotSafe { block_id: BlockId },
    #[error("No peers available. The committee size is {committee_size}")]
    NoPeersAvailable { committee_size: usize },
    #[error("Proposal validation error: {0}")]
    ProposalValidationError(#[from] ProposalValidationError),
    #[error("State tree error: {0}")]
    StateTreeError(#[from] tari_state_tree::StateTreeError),
    #[error("State root mismatch. Expected: {expected}, actual: {actual}")]
    StateRootMismatch { expected: Hash, actual: Hash },
}

impl CommsRpcConsensusSyncError {
    pub fn error_at_remote(self) -> Result<CommsRpcConsensusSyncError, CommsRpcConsensusSyncError> {
        match &self {
            CommsRpcConsensusSyncError::InvalidResponse(_) | CommsRpcConsensusSyncError::RpcError(_) => Err(self),
            _ => Ok(self),
        }
    }
}

impl From<CommsRpcConsensusSyncError> for HotStuffError {
    fn from(value: CommsRpcConsensusSyncError) -> Self {
        HotStuffError::SyncError(value.into())
    }
}

impl From<JmtStorageError> for CommsRpcConsensusSyncError {
    fn from(value: JmtStorageError) -> Self {
        Self::StateTreeError(value.into())
    }
}

impl From<RpcStatus> for CommsRpcConsensusSyncError {
    fn from(value: RpcStatus) -> Self {
        Self::RpcError(value.into())
    }
}
