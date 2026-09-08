//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::hotstuff::{HotStuffError, ProposalValidationError};
use tari_consensus_types::BlockId;
use tari_epoch_manager::EpochManagerError;
use tari_ootle_common_types::Epoch;
use tari_ootle_storage::{StorageError, consensus_models::TransactionPoolError};
use tari_rpc_framework::{RpcError, RpcStatus};
use tari_state_tree::{JmtStorageError, TreeHash};
use tari_validator_node_rpc::ValidatorNodeRpcClientError;

#[derive(Debug, thiserror::Error)]
pub enum RpcStateSyncError {
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
    #[error("Failed to sync from all {num_peers} peer(s)")]
    SyncFailedAllPeers { num_peers: usize },
    #[error("Proposal validation error: {0}")]
    ProposalValidationError(#[from] ProposalValidationError),
    #[error("State tree error: {0}")]
    StateTreeError(#[from] tari_state_tree::StateTreeError),
    #[error("State root mismatch. Expected: {expected}, actual: {actual}")]
    StateRootMismatch { expected: TreeHash, actual: TreeHash },
    #[error("Checkpoint for epoch {epoch} is not yet available from the previous committee")]
    CheckpointNotAvailable { epoch: Epoch },
    #[error("No committees found for epoch {0}")]
    NoCommittees(Epoch),
    #[error("Invariant error: {details}")]
    InvariantError { details: String },
}

impl RpcStateSyncError {
    pub fn error_at_remote(self) -> Result<RpcStateSyncError, RpcStateSyncError> {
        match &self {
            RpcStateSyncError::InvalidResponse(_) | RpcStateSyncError::RpcError(_) => Err(self),
            _ => Ok(self),
        }
    }
}

impl From<RpcStateSyncError> for HotStuffError {
    fn from(value: RpcStateSyncError) -> Self {
        HotStuffError::SyncError(value.into())
    }
}

impl From<JmtStorageError> for RpcStateSyncError {
    fn from(value: JmtStorageError) -> Self {
        Self::StateTreeError(value.into())
    }
}

impl From<RpcStatus> for RpcStateSyncError {
    fn from(value: RpcStatus) -> Self {
        Self::RpcError(value.into())
    }
}
