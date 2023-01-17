//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_dan_common_types::{Epoch, NodeHeight, PayloadId, ShardId, SubstateChange};
use tari_engine_types::commit_result::RejectReason;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::{
    services::{epoch_manager::EpochManagerError, PayloadProcessorError},
    storage::{shard_store::StoreError, StorageError},
};

#[derive(Error, Debug)]
pub enum HotStuffError {
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Received message from a node that is not in the committee")]
    ReceivedMessageFromNonCommitteeMember,
    #[error("Store error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Received invalid vote: {0}")]
    InvalidVote(String),
    #[error("Received invalid proposal: {0}")]
    InvalidProposal(#[from] ProposalValidationError),
    #[error("Send error")]
    SendError,
    #[error("Not the leader")]
    NotTheLeader,
    #[error("Payload failed to process: {0}")]
    PayloadProcessorError(#[from] PayloadProcessorError),
    #[error("Transaction rejected: {0}")]
    TransactionRejected(RejectReason),
    #[error("Storage Error: `{0}`")]
    StorageError(#[from] StorageError),
    #[error("Received generic message without node")]
    RecvProposalMessageWithoutNode,
    #[error("Shard has no data, when it was expected to")]
    ShardHasNoData,
    #[error("Invalid qc error: `{0}`")]
    InvalidQuorumCertificate(String),
    #[error("Failed to sign QC")]
    FailedToSignQc,
    #[error("This validator node is not included in the MMR")]
    ValidatorNodeNotIncludedInMmr,
    #[error("No committee for shard {shard} and epoch {epoch}")]
    NoCommitteeForShard { shard: ShardId, epoch: Epoch },
    #[error("Cannot vote on a proposal that has been rejected")]
    JustifyIsNotAccepted,
    #[error("Received NEWVIEW message without attached payload")]
    ReceivedNewViewWithoutPayload,
    #[error("Missing pledges: {}", .0.iter().map(|(s, c)| format!("{}: {}", s, c)).collect::<Vec<_>>().join(", "))]
    MissingPledges(Vec<(ShardId, SubstateChange)>),
    #[error("Shard {shard} already pledged to another payload {pledged_payload}, expected {expected}")]
    ShardPledgedToDifferentPayload {
        shard: ShardId,
        pledged_payload: PayloadId,
        expected: PayloadId,
    },
    #[error("Merkle proof error: {0}")]
    MerkleProofError(#[from] tari_mmr::MerkleProofError),
    #[error("Merkle mountain range error: {0}")]
    MerkleMountainRangeError(#[from] tari_mmr::error::MerkleMountainRangeError),
}

impl<T> From<mpsc::error::SendError<T>> for HotStuffError {
    fn from(_e: mpsc::error::SendError<T>) -> Self {
        Self::SendError
    }
}

#[derive(Error, Debug)]
pub enum ProposalValidationError {
    #[error(
        "Node payload height ({node_payload_height}) does not match justify payload height ({justify_payload_height})"
    )]
    NodePayloadHeightIncorrect {
        node_payload_height: NodeHeight,
        justify_payload_height: NodeHeight,
    },
    #[error("Node payload {node_payload} does not match justify payload {justify_payload}")]
    NodePayloadDoesNotMatchJustifyPayload {
        node_payload: PayloadId,
        justify_payload: PayloadId,
    },
    #[error("Node shard {node_shard} does not match justify shard {justify_shard}")]
    NodeShardDoesNotMatchJustifyPayload {
        node_shard: ShardId,
        justify_shard: ShardId,
    },
    #[error("Payload height is too high. Actual:{actual}, expected: {max}")]
    PayloadHeightIsTooHigh { actual: NodeHeight, max: NodeHeight },
    #[error("Local pledge was not provided in proposal")]
    LocalPledgeIsNone,
    #[error("Received proposal pledge for a different payload {pledged_payload} for shard {shard}")]
    PledgePayloadMismatch { shard: ShardId, pledged_payload: PayloadId },
}
