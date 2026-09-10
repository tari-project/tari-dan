//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::FixedHash;
use tari_consensus_types::{BlockId, Decision};
use tari_ootle_common_types::ShardGroup;
use tari_ootle_transaction::TransactionId;

use crate::consensus_models::TransactionPoolStage;

#[derive(Debug, Clone, thiserror::Error)]
pub enum NoVoteReason {
    #[error("Already voted at this height. Not voting again.")]
    AlreadyVotedAtHeight,
    #[error("Stage disagreement. Expected: {expected:?}, Actual: {stage:?}")]
    StageDisagreement {
        expected: TransactionPoolStage,
        stage: TransactionPoolStage,
    },
    #[error("Stage transaction not applicable. {current}->{next} output-only={is_output_only} decision: {decision}")]
    StageTransitionNotApplicable {
        current: TransactionPoolStage,
        next: TransactionPoolStage,
        is_output_only: bool,
        decision: Decision,
    },
    #[error(
        "Output only disagreement for local {shard_group}. Transaction ID: {transaction_id}, Stage: {stage}, Command: \
         {command}"
    )]
    OutputOnlyDisagreement {
        transaction_id: TransactionId,
        shard_group: ShardGroup,
        stage: TransactionPoolStage,
        command: &'static str,
    },
    #[error("The transaction is not in the pool")]
    TransactionNotInPool,
    #[error("Decision disagreement. Local: {local:?}, Remote: {remote:?}")]
    DecisionDisagreement { local: Decision, remote: Decision },
    #[error("Proposed invalid command SomeAccept(Commit, {transaction_id}) in {block_id}")]
    SomeAcceptAtomMustBeAbort {
        transaction_id: TransactionId,
        block_id: BlockId,
    },
    #[error("Proposed invalid command AllAccept(Abort, {transaction_id}) in {block_id}")]
    AllAcceptMustBeCommit {
        transaction_id: TransactionId,
        block_id: BlockId,
    },
    #[error("Fee disagreement")]
    FeeDisagreement,
    #[error("Leader fee disagreement")]
    LeaderFeeDisagreement,
    #[error("Total leader fee disagreement")]
    TotalLeaderFeeDisagreement,
    #[error("Total accumulated exhaust burn disagreement")]
    TotalExhaustBurnDisagreement,
    #[error("No leader fee")]
    NoLeaderFee,
    #[error("Local only proposed for multi shard")]
    LocalOnlyProposedForMultiShard,
    #[error("Multi shard proposed for local only")]
    MultiShardProposedForLocalOnly,
    #[error("Not all shard groups are prepared")]
    NotAllShardGroupsPrepared,
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
    #[error("The node is not at the end of the epoch")]
    NotEndOfEpoch,
    #[error("The node is not at the end of the epoch and other commands are present")]
    EndOfEpochWithOtherCommands,
    #[error("End-of-epoch next-epoch hash mismatch. Local oracle: {local}, proposed: {proposed}")]
    EndOfEpochHashMismatch { local: FixedHash, proposed: FixedHash },
    #[error(
        "End-of-epoch next-epoch hash cannot be ratified: local oracle has not observed the next epoch boundary block"
    )]
    EndOfEpochHashNotObserved,
    #[error("The state Merkle root does not match")]
    StateMerkleRootMismatch,
    #[error("The command Merkle root does not match")]
    CommandMerkleRootMismatch,
    #[error("Not all foreign input pledges are present")]
    NotAllForeignInputPledges,
    #[error("Not all inputs and outputs are accepted")]
    NotAllInputsOutputsAccepted,
    #[error("Invalid evidence")]
    InvalidEvidence { reason: InvalidEvidenceReason },
    #[error("Block transaction execution weight {total_weight} exceeds the maximum {max_weight}")]
    BlockWeightExceeded { total_weight: u64, max_weight: u64 },
    #[error("Block wasm execution points {total_points} exceed the maximum {max_points}")]
    BlockExecutionPointsExceeded { total_points: u64, max_points: u64 },
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum InvalidEvidenceReason {
    #[error("Expected evidence to contain {shard_group} but it was missing")]
    MissingInvolvedShardGroup { shard_group: ShardGroup },
    #[error("Evidence mismatched")]
    MismatchedEvidence,
    #[error("Not all shard groups prepared in evidence")]
    NotAllShardGroupsPrepared,
}

impl NoVoteReason {
    pub fn as_code_str(&self) -> &'static str {
        match self {
            Self::AlreadyVotedAtHeight => "ShouldNotVote",
            Self::StageDisagreement { .. } => "StageDisagreement",
            Self::StageTransitionNotApplicable { .. } => "StageTransitionNotApplicable",
            Self::OutputOnlyDisagreement { .. } => "OutputOnlyDisagreement",
            Self::TransactionNotInPool => "TransactionNotInPool",
            Self::DecisionDisagreement { .. } => "DecisionDisagreement",
            Self::SomeAcceptAtomMustBeAbort { .. } => "SomeAcceptAtomMustBeAbort",
            Self::AllAcceptMustBeCommit { .. } => "AllAcceptMustBeCommit",
            Self::FeeDisagreement => "FeeDisagreement",
            Self::LeaderFeeDisagreement => "LeaderFeeDisagreement",
            Self::TotalExhaustBurnDisagreement => "TotalExhaustBurnDisagreement",
            Self::NoLeaderFee => "NoLeaderFee",
            Self::LocalOnlyProposedForMultiShard => "LocalOnlyProposedForMultiShard",
            Self::MultiShardProposedForLocalOnly => "MultiShardProposedForLocalOnly",
            Self::NotAllShardGroupsPrepared => "NotAllShardGroupsPrepared",
            Self::ForeignProposalCommandInBlockMissing => "ForeignProposalCommandInBlockMissing",
            Self::ForeignProposalAlreadyProposed => "ForeignProposalAlreadyProposed",
            Self::ForeignProposalNotReceived => "ForeignProposalNotReceived",
            Self::ForeignProposalAlreadyConfirmed => "ForeignProposalAlreadyConfirmed",
            Self::ForeignProposalProcessingFailed => "ForeignProposalProcessingFailed",
            Self::NotEndOfEpoch => "NotEndOfEpoch",
            Self::EndOfEpochWithOtherCommands => "EndOfEpochWithOtherCommands",
            Self::EndOfEpochHashMismatch { .. } => "EndOfEpochHashMismatch",
            Self::EndOfEpochHashNotObserved => "EndOfEpochHashNotObserved",
            Self::TotalLeaderFeeDisagreement => "TotalLeaderFeeDisagreement",
            Self::StateMerkleRootMismatch => "StateMerkleRootMismatch",
            Self::CommandMerkleRootMismatch => "CommandMerkleRootMismatch",
            Self::NotAllForeignInputPledges => "NotAllForeignInputPledges",
            Self::NotAllInputsOutputsAccepted => "NotAllInputsOutputsAccepted",
            Self::InvalidEvidence { .. } => "InvalidEvidence",
            Self::BlockWeightExceeded { .. } => "BlockWeightExceeded",
            Self::BlockExecutionPointsExceeded { .. } => "BlockExecutionPointsExceeded",
        }
    }
}
