//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::Serialize;
use tari_dan_common_types::Epoch;
use tari_dan_storage::consensus_models::{Block, HighQc, QuorumCertificate};
use tari_transaction::Transaction;

#[derive(Debug, Clone, Serialize)]
pub struct SyncRequestMessage {
    pub epoch: Epoch,
    pub high_qc: HighQc,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncResponseMessage<TAddr> {
    pub epoch: Epoch,
    pub blocks: Vec<FullBlock<TAddr>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FullBlock<TAddr> {
    pub block: Block<TAddr>,
    pub qcs: Vec<QuorumCertificate<TAddr>>,
    pub transactions: Vec<Transaction>,
}
