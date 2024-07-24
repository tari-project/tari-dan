//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::consensus_models::{BlockId, LeafBlock};

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
    #[error("Leader timeout: new height {new_height}")]
    LeaderTimeout { new_height: NodeHeight },
    #[error("Block {block} has been parked ({num_missing_txs} missing, {num_awaiting_txs} awaiting execution)")]
    ProposedBlockParked {
        block: LeafBlock,
        num_missing_txs: usize,
        num_awaiting_txs: usize,
    },
    #[error("Parked block {block} is ready")]
    ParkedBlockReady { block: LeafBlock },
}
