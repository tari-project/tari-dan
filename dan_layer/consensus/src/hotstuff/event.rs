//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_dan_storage::consensus_models::BlockId;

#[derive(Debug, Clone)]
pub enum HotstuffEvent {
    BlockCommitted { block_id: BlockId },
    Failure { message: String },
}
