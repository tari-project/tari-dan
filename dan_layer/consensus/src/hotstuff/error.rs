//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::FixedHash;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{BlockId, LeafBlock, LockedBlock, QuorumCertificate, TransactionPoolError},
    StorageError,
};
use tari_epoch_manager::EpochManagerError;
use tari_mmr::BalancedBinaryMerkleProofError;
use tari_state_tree::StateTreeError;
use tari_transaction::{TransactionId, VersionedSubstateIdError};

use crate::{
    hotstuff::substate_store::SubstateStoreError,
    traits::{InboundMessagingError, OutboundMessagingError},
};

#[derive(Debug, thiserror::Error)]
pub enum HotStuffError {
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
    #[error("State tree error: {0}")]
    StateTreeError(#[from] StateTreeError),
    #[error("Internal channel send error when {context}")]
    InternalChannelClosed { context: &'static str },
    #[error("Inbound messaging error: {0}")]
    InboundMessagingError(#[from] InboundMessagingError),
    #[error("Outbound messaging error: {0}")]
    OutboundMessagingError(#[from] OutboundMessagingError),
    #[error("Epoch {epoch} is not active. {details}")]
    EpochNotActive { epoch: Epoch, details: String },
    #[error("Not registered for current epoch {epoch}")]
    NotRegisteredForCurrentEpoch { epoch: Epoch },
    #[error("Received message from non-committee member. Epoch: {epoch}, Sender: {sender}, {context}")]
    ReceivedMessageFromNonCommitteeMember {
        epoch: Epoch,
        sender: String,
        context: String,
    },
    #[error("Proposal validation error: {0}")]
    ProposalValidationError(#[from] ProposalValidationError),
    #[error("Decision mismatch for block {block_id} in pool {pool}")]
    DecisionMismatch { block_id: BlockId, pool: &'static str },
    #[error("Not the leader. {details}")]
    NotTheLeader { details: String },
    #[error("Merkle proof error: {0}")]
    BalancedBinaryMerkleProofError(#[from] BalancedBinaryMerkleProofError),
    #[error("Epoch manager error: {0}")]
    EpochManagerError(anyhow::Error),
    #[error("State manager error: {0}")]
    StateManagerError(anyhow::Error),
    #[error("Invalid vote signature from {signer_public_key} (unauthenticated)")]
    InvalidVoteSignature { signer_public_key: String },
    #[error("Vote sent from peer {address} did not match the expected signer public key {signer_public_key}")]
    RejectingVoteNotSentBySigner { address: String, signer_public_key: String },
    #[error("Transaction pool error: {0}")]
    TransactionPoolError(#[from] TransactionPoolError),
    #[error("Transaction {transaction_id} does not exist")]
    TransactionDoesNotExist { transaction_id: TransactionId },
    #[error(
        "Unable execute block {block_id} because the committee decided to ACCEPT transaction {transaction_id} but it \
         failed to execute locally: {reject_reason}"
    )]
    RejectedTransactionCommitDecision {
        block_id: BlockId,
        transaction_id: TransactionId,
        reject_reason: String,
    },
    #[error("Pacemaker channel dropped: {details}")]
    PacemakerChannelDropped { details: String },
    #[error(
        "Bad new view message: HighQC height {high_qc_height}, received new height {received_new_height}: {details}"
    )]
    BadNewViewMessage {
        high_qc_height: NodeHeight,
        received_new_height: NodeHeight,
        details: String,
    },
    #[error("BUG Invariant error occurred: {0}")]
    InvariantError(String),
    #[error("Sync error: {0}")]
    SyncError(anyhow::Error),
    #[error("Fallen behind: local_height={local_height}, qc_height={qc_height}")]
    FallenBehind {
        local_height: NodeHeight,
        qc_height: NodeHeight,
    },
    #[error("Transaction executor error: {0}")]
    TransactionExecutorError(String),
    #[error("Invalid sync request: {details}")]
    InvalidSyncRequest { details: String },
    #[error("Some input versions were not resolved at execution time: {0}")]
    VersionedSubstateIdError(#[from] VersionedSubstateIdError),
    #[error("Substate store error: {0}")]
    SubstateStoreError(#[from] SubstateStoreError),
}

impl From<EpochManagerError> for HotStuffError {
    fn from(err: EpochManagerError) -> Self {
        Self::EpochManagerError(err.into())
    }
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
    #[error("Node proposed by {proposed_by} with hash {hash} is missing foreign index")]
    MissingForeignCounters { proposed_by: String, hash: BlockId },
    #[error("Node proposed by {proposed_by} with hash {hash} has invalid foreign counters: {details}")]
    InvalidForeignCounters {
        proposed_by: String,
        hash: BlockId,
        details: String,
    },
    #[error("Node proposed by {proposed_by} with hash {hash} is the genesis block")]
    ProposingGenesisBlock { proposed_by: String, hash: BlockId },
    #[error("Justification block {justify_block} for proposed block {block_description} by {proposed_by} not found")]
    JustifyBlockNotFound {
        proposed_by: String,
        block_description: String,
        justify_block: BlockId,
    },
    #[error("QC in block {block_id} that was proposed by {proposed_by} is invalid: {details}")]
    JustifyBlockInvalid {
        proposed_by: String,
        block_id: BlockId,
        details: String,
    },
    #[error("Candidate block {candidate_block_height} is not higher than justify {justify_block_height}")]
    CandidateBlockNotHigherThanJustify {
        justify_block_height: NodeHeight,
        candidate_block_height: NodeHeight,
    },
    #[error(
        "Candidate block {candidate_block_height} is higher than max failures {max_failures}. Proposed by \
         {proposed_by}, justify block height {justify_block_height}"
    )]
    CandidateBlockHigherThanMaxFailures {
        proposed_by: String,
        justify_block_height: NodeHeight,
        candidate_block_height: NodeHeight,
        max_failures: usize,
    },
    #[error("Candidate block {candidate_block_height} does not extend justify block {justify_block_height}")]
    CandidateBlockDoesNotExtendJustify {
        justify_block_height: NodeHeight,
        candidate_block_height: NodeHeight,
    },
    #[error("Block {block_id} proposed by {proposed_by} is not the leader. Expect {expected_leader}")]
    NotLeader {
        proposed_by: String,
        expected_leader: String,
        block_id: BlockId,
    },
    #[error(
        "Block {candidate_block} justify proposed by {proposed_by} is less than the current locked {locked_block}"
    )]
    CandidateBlockNotHigherThanLockedBlock {
        proposed_by: String,
        locked_block: LockedBlock,
        candidate_block: LeafBlock,
    },
    #[error("Proposed block {block_id} {height} already has been processed")]
    BlockAlreadyProcessed { block_id: BlockId, height: NodeHeight },
    #[error("Proposed block {block_id} {height} doesn't have a signature")]
    MissingSignature { block_id: BlockId, height: NodeHeight },
    #[error("Proposed block {block_id} {height} has invalid signature")]
    InvalidSignature { block_id: BlockId, height: NodeHeight },
    #[error("QC is not valid: {qc}")]
    QCisNotValid { qc: QuorumCertificate },
    #[error("QC has invalid signature: {qc}")]
    QCInvalidSignature { qc: QuorumCertificate },
    #[error("Quorum was not reached: {qc}")]
    QuorumWasNotReached { qc: QuorumCertificate },
    #[error("Merkle proof error: {0}")]
    BalancedBinaryMerkleProofError(#[from] BalancedBinaryMerkleProofError),
    #[error("Invalid network in block {block_id}: expected {expected_network}, given {block_network}")]
    InvalidNetwork {
        expected_network: String,
        block_network: String,
        block_id: BlockId,
    },
    #[error("Invalid state merkle root for block {block_id}: calculated {calculated} but block has {from_block}")]
    InvalidStateMerkleRoot {
        block_id: BlockId,
        calculated: FixedHash,
        from_block: FixedHash,
    },
    #[error("Problem converting values")]
    QCConversionError,
    #[error("Validator {validator} is not in committee for shard {expected_shard}. Actual shard: {actual_shard}")]
    ValidatorNotInCommittee {
        validator: String,
        expected_shard: String,
        actual_shard: String,
    },
    #[error("Base layer block hash for block with height {proposed} too high, current height {current}")]
    BlockHeightTooHigh { proposed: u64, current: u64 },
    #[error("Base layer block hash for block with height {proposed} too small, current height {current}")]
    BlockHeightTooSmall { proposed: u64, current: u64 },
    #[error("Base layer block hash ({hash}) is not known to the node")]
    BlockHashNotFound { hash: FixedHash },
    #[error("Base layer block height {height} does not match the real height {real_height}")]
    BlockHeightMismatch { height: u64, real_height: u64 },
    #[error("Base layer block with height {base_layer_block_height} is not the last block of the epoch")]
    NotLastBlockOfEpoch {
        block_id: BlockId,
        base_layer_block_height: u64,
    },
}
