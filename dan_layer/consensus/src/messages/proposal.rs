//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use serde::Serialize;
use tari_dan_storage::consensus_models::{
    Block,
    BlockPledge,
    ForeignParkedProposal,
    ForeignProposal,
    QuorumCertificate,
};

#[derive(Debug, Clone, Serialize)]
pub struct ProposalMessage {
    pub block: Block,
    pub foreign_proposals: Vec<ForeignProposal>,
}

impl Display for ProposalMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProposalMessage({})", self.block)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ForeignProposalMessage {
    pub block: Block,
    pub justify_qc: QuorumCertificate,
    pub block_pledge: BlockPledge,
}

impl From<ForeignProposalMessage> for ForeignParkedProposal {
    fn from(msg: ForeignProposalMessage) -> Self {
        ForeignParkedProposal::new(msg.into())
    }
}

impl From<ForeignProposalMessage> for ForeignProposal {
    fn from(msg: ForeignProposalMessage) -> Self {
        ForeignProposal::new(msg.block, msg.block_pledge, msg.justify_qc)
    }
}

impl From<ForeignProposal> for ForeignProposalMessage {
    fn from(proposal: ForeignProposal) -> Self {
        ForeignProposalMessage {
            block: proposal.block,
            justify_qc: proposal.justify_qc,
            block_pledge: proposal.block_pledge,
        }
    }
}

impl From<ForeignParkedProposal> for ForeignProposalMessage {
    fn from(proposal: ForeignParkedProposal) -> Self {
        let proposal = proposal.into_proposal();
        ForeignProposalMessage {
            block: proposal.block,
            justify_qc: proposal.justify_qc,
            block_pledge: proposal.block_pledge,
        }
    }
}

impl Display for ForeignProposalMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ForeignProposalMessage({}, {}, {} pledged substate(s))",
            self.block,
            self.justify_qc,
            self.block_pledge.num_substates_pledged()
        )
    }
}
