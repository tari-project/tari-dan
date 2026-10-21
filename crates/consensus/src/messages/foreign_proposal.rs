//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use serde::Serialize;
use tari_consensus_types::BlockId;
use tari_ootle_common_types::{Epoch, ShardGroup};
use tari_ootle_storage::consensus_models::{ForeignParkedProposal, ForeignProposal, ForeignProposalRecord};

#[derive(Debug, Clone, Serialize)]
pub struct ForeignProposalMessage {
    pub proposal: Box<ForeignProposal>,
}

impl From<ForeignProposalMessage> for ForeignParkedProposal {
    fn from(msg: ForeignProposalMessage) -> Self {
        let (commit_proof, block_pledge, block_metadata) = msg.proposal.into_parts();
        ForeignParkedProposal::new(commit_proof, block_pledge, block_metadata)
    }
}

impl From<ForeignProposalMessage> for ForeignProposalRecord {
    fn from(msg: ForeignProposalMessage) -> Self {
        ForeignProposalRecord::new(*msg.proposal)
    }
}

impl From<ForeignProposal> for ForeignProposalMessage {
    fn from(proposal: ForeignProposal) -> Self {
        ForeignProposalMessage {
            proposal: Box::new(proposal),
        }
    }
}

impl From<ForeignParkedProposal> for ForeignProposalMessage {
    fn from(proposal: ForeignParkedProposal) -> Self {
        let proposal = proposal.into_proposal();
        ForeignProposalMessage {
            proposal: Box::new(proposal),
        }
    }
}

impl Display for ForeignProposalMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ForeignProposalMessage({})", self.proposal,)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ForeignProposalNotificationMessage {
    pub block_id: BlockId,
    pub epoch: Epoch,
    /// The shard group whose committee produced the block in `epoch`. Receivers request the foreign proposal from this
    /// committee, whichever peer relayed the notification to them.
    pub shard_group: ShardGroup,
    /// The shard groups this notification is intended for. The notification is gossiped on a single network-wide
    /// topic, so every validator receives it; only validators whose shard group is listed here need to fetch the
    /// foreign proposal.
    pub shard_groups: Vec<ShardGroup>,
}

impl Display for ForeignProposalNotificationMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ForeignProposalNotificationMessage({}, {}, {}, [{}])",
            self.epoch,
            self.shard_group,
            self.block_id,
            self.shard_groups
                .iter()
                .map(|sg| sg.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum ForeignProposalRequestMessage {
    /// Request a foreign proposal for a specific block ID in response to a ForeignProposalNotificationMessage that has
    /// not already been received.
    ByBlockId {
        block_id: BlockId,
        /// The shard group to return pledges for
        for_shard_group: ShardGroup,
        epoch: Epoch,
    },
}

impl ForeignProposalRequestMessage {
    pub fn epoch(&self) -> Epoch {
        match self {
            Self::ByBlockId { epoch, .. } => *epoch,
        }
    }
}

impl Display for ForeignProposalRequestMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ByBlockId {
                block_id,
                for_shard_group,
                epoch,
            } => {
                write!(
                    f,
                    "ForeignProposalRequestMessage(ByBlockId, {}, {}, {})",
                    epoch, for_shard_group, block_id
                )
            },
        }
    }
}
