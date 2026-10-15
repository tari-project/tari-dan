//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_network::Network;
use tari_ootle_common_types::{Epoch, ShardGroup, shard::Shard};
use tari_ootle_storage::{
    SubstateProofVerifyError,
    consensus_models::{CommittedBlockProofError, EpochCheckpointValidationError},
};
use tari_state_tree::ValidatorSetProofError;

#[derive(Debug, thiserror::Error)]
pub enum LightClientError {
    #[error("Failed to decode the {what}: {details}")]
    Decode { what: &'static str, details: String },
    #[error("The {what} is for network byte {found}, but this light client follows {expected}")]
    WrongNetwork {
        what: &'static str,
        expected: Network,
        found: u8,
    },
    #[error("The committee proof is for {found}, but the validator set it is checked against is for {expected}")]
    CommitteeEpochMismatch { expected: Epoch, found: Epoch },
    #[error("The committee proof does not prove a committee of the trusted validator set: {0}")]
    CommitteeNotInSet(#[from] ValidatorSetProofError),
    #[error("The committee proof proves an empty committee for shard group {shard_group} in {epoch}")]
    EmptyCommittee { epoch: Epoch, shard_group: ShardGroup },
    #[error("The committee proof is for shard group {shard_group}, which does not contain the walked shard {shard}")]
    CommitteeOutsideWalkedShard { shard_group: ShardGroup, shard: Shard },
    #[error("The checkpoint ends {found}, but the trusted validator set is for {expected}")]
    CheckpointEpochMismatch { expected: Epoch, found: Epoch },
    #[error(
        "The checkpoint for {epoch} is of shard group {found}, but the trusted committee is of shard group {expected}"
    )]
    CheckpointShardGroupMismatch {
        epoch: Epoch,
        expected: ShardGroup,
        found: ShardGroup,
    },
    #[error("The checkpoint for {epoch} does not verify: {source}")]
    InvalidCheckpoint {
        epoch: Epoch,
        source: EpochCheckpointValidationError,
    },
    /// The quorum certificate does not cover the header metadata preimage, so whoever serves a checkpoint can strip it.
    #[error(
        "The checkpoint for {epoch} carries no header metadata preimage naming the next validator set, so it proves \
         no handover"
    )]
    NoNextValidatorSet { epoch: Epoch },
    #[error("The checkpoint for {epoch} came without the next epoch's committee")]
    MissingNextCommittee { epoch: Epoch },
    #[error("The commit proof is from {found}, but the committee it is checked against is of {expected}")]
    CommitProofEpochMismatch { expected: Epoch, found: Epoch },
    #[error("The commit proof is of shard group {found}, but the committee it is checked against is of {expected}")]
    CommitProofShardGroupMismatch { expected: ShardGroup, found: ShardGroup },
    #[error("The commit proof does not verify: {0}")]
    InvalidCommitProof(#[from] CommittedBlockProofError),
    #[error("The substate proof does not verify: {0}")]
    InvalidSubstateProof(#[from] SubstateProofVerifyError),
    #[error("The indexer served the receipt of transaction {transaction_id} without a proof")]
    MissingReceiptProof { transaction_id: String },
    #[error("The substate diff does not match the proven receipt: {details}")]
    DiffDoesNotMatchReceipt { details: String },
}
