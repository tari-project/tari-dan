//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::PublicKey;
use tari_dan_common_types::{Epoch, ShardId};

#[derive(thiserror::Error, Debug)]
pub enum EpochManagerError {
    #[error("Could not receive from channel")]
    ReceiveError,
    #[error("Could not send to channel")]
    SendError,
    #[error("Base node errored: {0}")]
    BaseNodeError(anyhow::Error),
    #[error("No epoch found {0:?}")]
    NoEpochFound(Epoch),
    #[error("No committee found for shard {0:?}")]
    NoCommitteeFound(ShardId),
    #[error("Unexpected request")]
    UnexpectedRequest,
    #[error("Unexpected response")]
    UnexpectedResponse,
    #[error("SQLite Storage error: {0}")]
    SqlLiteStorageError(anyhow::Error),
    #[error("No validator nodes found for current shard key")]
    ValidatorNodesNotFound,
    // #[error("Rpc error: {0}")]
    // RpcError(#[from] RpcError),
    // #[error("Rpc status error: {0}")]
    // RpcStatus(#[from] RpcStatus),
    #[error("No committee VNs found for shard {shard_id} and epoch {epoch}")]
    NoCommitteeVns { shard_id: ShardId, epoch: Epoch },
    #[error("Validator node is not registered")]
    ValidatorNodeNotRegistered,
    #[error("Base layer consensus constants not set")]
    BaseLayerConsensusConstantsNotSet,
    #[error("Base layer could not return shard key for {public_key} at height {block_height}")]
    ShardKeyNotFound { public_key: PublicKey, block_height: u64 },
    #[error("BalancedBinaryMerkleTreeError: {0}")]
    BalancedBinaryMerkleTreeError(anyhow::Error),
    #[error("BalancedBinaryMerkleProofError: {0}")]
    BalancedBinaryMerkleProofError(anyhow::Error),
    #[error("Integer overflow: {func}")]
    IntegerOverflow { func: &'static str },
}
