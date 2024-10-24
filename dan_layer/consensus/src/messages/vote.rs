//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use serde::Serialize;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::consensus_models::{BlockId, LastSentVote, QuorumDecision, ValidatorSignature};

#[derive(Debug, Clone, Serialize)]
pub struct VoteMessage {
    pub epoch: Epoch,
    pub block_id: BlockId,
    /// The purported height of the block that this vote is for. This is informational only (on_inbound_message) and
    /// should never be relied upon for any other purpose until it is validated. We may receive
    /// a vote before the block, and we don't retrospectively check the height of all the votes after the block is
    /// received.
    pub unverified_block_height: NodeHeight,
    pub decision: QuorumDecision,
    pub signature: ValidatorSignature,
}

impl From<LastSentVote> for VoteMessage {
    fn from(value: LastSentVote) -> Self {
        Self {
            epoch: value.epoch,
            block_id: value.block_id,
            unverified_block_height: value.block_height,
            decision: value.decision,
            signature: value.signature,
        }
    }
}

impl Display for VoteMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "VoteMessage: {}, block_id: {}, {}, decision: {:?}, voter: {:?}",
            self.epoch, self.block_id, self.unverified_block_height, self.decision, self.signature.public_key
        )
    }
}
