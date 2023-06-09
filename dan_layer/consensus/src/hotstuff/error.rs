//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::Epoch;
use tari_dan_storage::{consensus_models::BlockId, StorageError};

#[derive(Debug, thiserror::Error)]
pub enum HotStuffError {
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
    #[error("Internal channel send error when {context}")]
    InternalChannelClosed { context: &'static str },
    #[error("Epoch {epoch} is not active. {context}")]
    EpochNotActive { epoch: Epoch, context: String },
    #[error("Received message from non-committee member. Epoch: {epoch}, Sender: {sender}, {context}")]
    ReceivedMessageFromNonCommitteeMember {
        epoch: Epoch,
        sender: String,
        context: String,
    },
    #[error("Proposal validation error: {0}")]
    ProposalValidationError(#[from] ProposalValidationError),
}

#[derive(Debug, thiserror::Error)]
pub enum ProposalValidationError {
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
    #[error("Node proposed by {proposed_by} with hash {hash} does not match calculated hash {calculated_hash}")]
    NodeHashMismatch {
        proposed_by: String,
        hash: BlockId,
        calculated_hash: BlockId,
    },
    #[error("Node proposed by {proposed_by} with hash {hash} did not satisfy the safeNode predicate")]
    NotSafeBlock { proposed_by: String, hash: BlockId },
    #[error("Node proposed by {proposed_by} with hash {hash} did not satisfy the validNode predicate")]
    ProposingGenesisBlock { proposed_by: String, hash: BlockId },
    #[error("Justification block {justify_block} for proposed block {hash} by {proposed_by} not found")]
    JustifyBlockNotFound {
        proposed_by: String,
        hash: BlockId,
        justify_block: BlockId,
    },
    #[error("QC in block {block_id} proposed by {proposed_by} is invalid: {details}")]
    JustifyBlockInvalid {
        proposed_by: String,
        block_id: BlockId,
        details: String,
    },
}
