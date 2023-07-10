//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::Serialize;
use tari_dan_common_types::Epoch;
use tari_dan_storage::consensus_models::{BlockId, ExecutedTransaction};

#[derive(Debug, Clone, Serialize)]
pub struct RequestedTransactionMessage {
    pub epoch: Epoch,
    pub block_id: BlockId,
    pub transactions: Vec<ExecutedTransaction>,
}
