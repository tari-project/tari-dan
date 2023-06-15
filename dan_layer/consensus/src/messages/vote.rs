//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common_types::types::Signature;
use tari_dan_common_types::Epoch;
use tari_dan_storage::consensus_models::BlockId;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VoteMessage {
    pub epoch: Epoch,
    pub block_id: BlockId,
    pub decision: QuorumDecision,
    pub signature: Signature,
    // TODO
    // pub merkle_proof: BalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake256>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum QuorumDecision {
    Accept,
    Reject(QuorumRejectReason),
}

impl QuorumDecision {
    pub fn is_reject(&self) -> bool {
        matches!(self, QuorumDecision::Reject(_))
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum QuorumRejectReason {
    // TODO: work out reject reasons
    TransactionPoolsDisagree,
}

impl QuorumRejectReason {
    pub fn as_u8(&self) -> u8 {
        match self {
            QuorumRejectReason::TransactionPoolsDisagree => 1,
        }
    }
}

impl QuorumDecision {
    pub fn as_u8(&self) -> u8 {
        match self {
            QuorumDecision::Accept => 0,
            QuorumDecision::Reject(reason) => reason.as_u8(),
        }
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(QuorumDecision::Accept),
            1 => Some(QuorumDecision::Reject(QuorumRejectReason::TransactionPoolsDisagree)),
            _ => None,
        }
    }
}
