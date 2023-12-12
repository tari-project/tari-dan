//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::Serialize;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::consensus_models::{BlockId, LastSentVote, QuorumDecision, ValidatorSignature};

#[derive(Debug, Clone, Serialize)]
pub struct VoteMessage {
    pub epoch: Epoch,
    pub block_id: BlockId,
    pub block_height: NodeHeight,
    pub decision: QuorumDecision,
    pub signature: ValidatorSignature,
}

impl From<LastSentVote> for VoteMessage {
    fn from(value: LastSentVote) -> Self {
        Self {
            epoch: value.epoch,
            block_id: value.block_id,
            block_height: value.block_height,
            decision: value.decision,
            signature: value.signature,
        }
    }
}
