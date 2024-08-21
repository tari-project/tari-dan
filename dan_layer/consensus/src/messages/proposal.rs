//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use serde::Serialize;
use tari_dan_storage::consensus_models::{Block, BlockPledge, ForeignParkedProposal, QuorumCertificate};

#[derive(Debug, Clone, Serialize)]
pub struct ProposalMessage {
    pub block: Block,
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
        ForeignParkedProposal::new(msg.block, msg.justify_qc, msg.block_pledge)
    }
}

impl From<ForeignParkedProposal> for ForeignProposalMessage {
    fn from(block: ForeignParkedProposal) -> Self {
        ForeignProposalMessage {
            block: block.block,
            justify_qc: block.justify_qc,
            block_pledge: block.block_pledge,
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
