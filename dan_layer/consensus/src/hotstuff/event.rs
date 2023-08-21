//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_dan_storage::consensus_models::BlockId;

#[derive(Debug, Clone)]
pub enum HotstuffEvent {
    /// A block has been committed
    BlockCommitted { block_id: BlockId },
    /// A critical failure occurred in consensus
    Failure { message: String },
}
