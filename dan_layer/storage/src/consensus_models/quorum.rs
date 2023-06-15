//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};

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
