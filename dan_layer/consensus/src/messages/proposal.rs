//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use serde::Serialize;
use tari_dan_storage::consensus_models::{Block, BlockPledge, QuorumCertificate};

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
