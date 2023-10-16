//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::Serialize;
use tari_dan_common_types::{hashing::ValidatorNodeMerkleProof, Epoch, NodeHeight};
use tari_dan_storage::consensus_models::{BlockId, LastSentVote, QuorumDecision, ValidatorSignature};

#[derive(Debug, Clone, Serialize)]
pub struct VoteMessage<TAddr> {
    pub epoch: Epoch,
    pub block_id: BlockId,
    pub block_height: NodeHeight,
    pub decision: QuorumDecision,
    pub signature: ValidatorSignature<TAddr>,
    // TODO: Surely the current leader can generate the aggregate proof for the new QC. I dont think we need to include
    //       it in each vote message.
    pub merkle_proof: ValidatorNodeMerkleProof,
}

impl<TAddr> From<LastSentVote<TAddr>> for VoteMessage<TAddr> {
    fn from(value: LastSentVote<TAddr>) -> Self {
        Self {
            epoch: value.epoch,
            block_id: value.block_id,
            block_height: value.block_height,
            decision: value.decision,
            signature: value.signature,
            merkle_proof: value.merkle_proof,
        }
    }
}
