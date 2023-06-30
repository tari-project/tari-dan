//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{hashing::ValidatorNodeMerkleProof, Epoch};
use tari_dan_storage::consensus_models::{BlockId, QuorumDecision, ValidatorSignature};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VoteMessage {
    pub epoch: Epoch,
    pub block_id: BlockId,
    pub decision: QuorumDecision,
    pub signature: ValidatorSignature,
    pub merkle_proof: ValidatorNodeMerkleProof,
}
