//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{hashing::ValidatorNodeMerkleProof, Epoch, NodeHeight};
use tari_dan_storage::consensus_models::{BlockId, QuorumDecision, ValidatorSignature};

#[derive(Debug, Clone, Deserialize, Serialize)]
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
