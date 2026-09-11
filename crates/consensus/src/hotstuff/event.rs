//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_consensus_types::{BlockId, LeafBlock};
use tari_ootle_common_types::{Epoch, NodeHeight, ShardGroup};

#[derive(Debug, Clone, thiserror::Error)]
pub enum HotstuffEvent {
    #[error("Block {block_id} has been committed for epoch {epoch} at height {height}")]
    BlockCommitted {
        epoch: Epoch,
        block_id: BlockId,
        height: NodeHeight,
    },
    #[error("Consensus failure: {message}")]
    Failure { message: String },
    /// This node is behind its peers and is leaving consensus to sync. A designed transition, not a failure.
    #[error("Sync required: {message}")]
    SyncRequired { message: String },
    #[error("Leader timeout: height {height}")]
    LeaderTimeout { height: NodeHeight },
    #[error("Block {block} has been parked ({num_missing_txs} missing, {num_awaiting_txs} awaiting execution)")]
    ProposedBlockParked {
        block: LeafBlock,
        num_missing_txs: usize,
        num_awaiting_txs: usize,
    },
    #[error("Parked block {block} is ready")]
    ParkedBlockReady { block: LeafBlock },
    #[error("Epoch changed to {epoch}")]
    EpochChanged {
        epoch: Epoch,
        registered_shard_group: Option<ShardGroup>,
    },
}
