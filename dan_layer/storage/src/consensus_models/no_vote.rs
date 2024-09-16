//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::consensus_models::{Decision, TransactionPoolStage};

#[derive(Debug, Clone, thiserror::Error)]
pub enum NoVoteReason {
    #[error("The node should not vote")]
    ShouldNotVote,
    #[error("Stage disagreement. Expected: {expected:?}, Actual: {stage:?}")]
    StageDisagreement {
        expected: TransactionPoolStage,
        stage: TransactionPoolStage,
    },
    #[error("The transaction is not in the pool")]
    TransactionNotInPool,
    #[error("Decision disagreement. Local: {local:?}, Remote: {remote:?}")]
    DecisionDisagreement { local: Decision, remote: Decision },
    #[error("Fee disagreement")]
    FeeDisagreement,
    #[error("Leader fee disagreement")]
    LeaderFeeDisagreement,
    #[error("Total leader fee disagreement")]
    TotalLeaderFeeDisagreement,
    #[error("No leader fee")]
    NoLeaderFee,
    #[error("Local only proposed for multi shard")]
    LocalOnlyProposedForMultiShard,
    #[error("Multi shard proposed for local only")]
    MultiShardProposedForLocalOnly,
    #[error("Not all inputs prepared")]
    NotAllInputsPrepared,
    #[error("Foreign proposal command in block missing")]
    ForeignProposalCommandInBlockMissing,
    #[error("Foreign proposal already proposed")]
    ForeignProposalAlreadyProposed,
    #[error("Foreign proposal not received")]
    ForeignProposalNotReceived,
    #[error("Foreign proposal already confirmed")]
    ForeignProposalAlreadyConfirmed,
    #[error("Foreign proposal processing failed")]
    ForeignProposalProcessingFailed,
    #[error("Mint confidential output unknown")]
    MintConfidentialOutputUnknown,
    #[error("Mint confidential output store failed")]
    MintConfidentialOutputStoreFailed,
    #[error("The node is not at the end of the epoch")]
    NotEndOfEpoch,
    #[error("The node is not at the end of the epoch and other commands are present")]
    EndOfEpochWithOtherCommands,
    #[error("The Merkle root does not match")]
    MerkleRootMismatch,
}

impl NoVoteReason {
    pub fn as_code_str(&self) -> &'static str {
        match self {
            Self::ShouldNotVote => "ShouldNotVote",
            Self::StageDisagreement { .. } => "StageDisagreement",
            Self::TransactionNotInPool => "TransactionNotInPool",
            Self::DecisionDisagreement { .. } => "DecisionDisagreement",
            Self::FeeDisagreement => "FeeDisagreement",
            Self::LeaderFeeDisagreement => "LeaderFeeDisagreement",
            Self::NoLeaderFee => "NoLeaderFee",
            Self::LocalOnlyProposedForMultiShard => "LocalOnlyProposedForMultiShard",
            Self::MultiShardProposedForLocalOnly => "MultiShardProposedForLocalOnly",
            Self::NotAllInputsPrepared => "NotAllInputsPrepared",
            Self::ForeignProposalCommandInBlockMissing => "ForeignProposalCommandInBlockMissing",
            Self::ForeignProposalAlreadyProposed => "ForeignProposalAlreadyProposed",
            Self::ForeignProposalNotReceived => "ForeignProposalNotReceived",
            Self::ForeignProposalAlreadyConfirmed => "ForeignProposalAlreadyConfirmed",
            Self::ForeignProposalProcessingFailed => "ForeignProposalProcessingFailed",
            Self::MintConfidentialOutputUnknown => "MintConfidentialOutputUnknown",
            Self::MintConfidentialOutputStoreFailed => "MintConfidentialOutputStoreFailed",
            Self::NotEndOfEpoch => "NotEndOfEpoch",
            Self::EndOfEpochWithOtherCommands => "EndOfEpochWithOtherCommands",
            Self::TotalLeaderFeeDisagreement => "TotalLeaderFeeDisagreement",
            Self::MerkleRootMismatch => "MerkleRootMismatch",
        }
    }
}
