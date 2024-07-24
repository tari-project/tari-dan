//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use serde::Serialize;
use tari_dan_common_types::Epoch;
use tari_dan_storage::consensus_models::BlockId;
use tari_transaction::TransactionId;

#[derive(Debug, Clone, Serialize)]
pub struct MissingTransactionsRequest {
    pub request_id: u32,
    pub epoch: Epoch,
    pub block_id: BlockId,
    pub transactions: HashSet<TransactionId>,
}
