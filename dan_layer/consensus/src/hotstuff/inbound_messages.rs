//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeMap, VecDeque},
    fmt::{Display, Formatter},
};

use log::*;
use tari_dan_common_types::{NodeAddressable, NodeHeight};
use tari_dan_storage::consensus_models::QuorumCertificate;
use tokio::sync::mpsc;

use crate::messages::{ProposalMessage, VoteMessage};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::inbound_hotstuff_messages";

pub struct InboundHotstuffMessages<TAddr> {
    rx_incoming: mpsc::UnboundedReceiver<ProposalOrVote<TAddr>>,
    tx_incoming: mpsc::UnboundedSender<ProposalOrVote<TAddr>>,
    buffer: BTreeMap<NodeHeight, VecDeque<ProposalOrVote<TAddr>>>,
}

impl<TAddr: NodeAddressable> InboundHotstuffMessages<TAddr> {
    pub fn new() -> Self {
        let (tx_incoming, rx_incoming) = mpsc::unbounded_channel();
        Self {
            rx_incoming,
            tx_incoming,
            buffer: BTreeMap::new(),
        }
    }

    pub async fn next(&mut self, current_height: NodeHeight) -> Result<ProposalOrVote<TAddr>, NeedsSync> {
        // Clear buffer with lower heights
        self.buffer = self.buffer.split_off(&current_height);

        // Check if message is in the buffer
        if let Some(buffer) = self.buffer.get_mut(&current_height) {
            if let Some(msg) = buffer.pop_front() {
                return Ok(msg);
            }
        }

        loop {
            let msg = self
                .rx_incoming
                .recv()
                .await
                .expect("sender exists in this struct, so never be dropped before the receiver");
            if let Some(justify) = msg.justify() {
                if justify.block_height() > current_height {
                    return Err(NeedsSync);
                }
            }

            match msg.height() {
                // Discard old message
                h if h < current_height => {
                    debug!(target: LOG_TARGET, "Discard message {} is for previous height {}. Current height {}", msg, h, current_height);
                    continue;
                },
                // Buffer message for future height
                h if h > current_height => {
                    debug!(target: LOG_TARGET, "Message {} is for future block {}. Current height {}", msg, h, current_height);
                    self.push_to_buffer(msg);
                    continue;
                },
                // Height is current, return message
                _ => return Ok(msg),
            }
        }
    }

    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
    }

    pub fn enqueue(&self, msg: ProposalOrVote<TAddr>) {
        debug!(target: LOG_TARGET, "Enqueue message {}", msg);
        self.tx_incoming
            .send(msg)
            .expect("receiver exists in this struct, so never be dropped before the sender");
    }

    fn push_to_buffer(&mut self, msg: ProposalOrVote<TAddr>) {
        self.buffer.entry(msg.height()).or_default().push_back(msg);
    }
}

pub enum ProposalOrVote<TAddr> {
    Proposal(ProposalMessage<TAddr>),
    Vote(VoteMessage<TAddr>),
}

impl<TAddr> ProposalOrVote<TAddr> {
    pub fn justify(&self) -> Option<&QuorumCertificate<TAddr>> {
        match self {
            ProposalOrVote::Proposal(msg) => Some(msg.block.justify()),
            ProposalOrVote::Vote(_) => None,
        }
    }

    pub fn height(&self) -> NodeHeight {
        match self {
            ProposalOrVote::Proposal(msg) => msg.block.height(),
            // If current height is 2, then we are listening for votes for 2 at current height 3
            ProposalOrVote::Vote(msg) => msg.block_height.saturating_add(NodeHeight(1)),
        }
    }
}

impl<TAddr: NodeAddressable> Display for ProposalOrVote<TAddr> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ProposalOrVote::Proposal(msg) => write!(f, "Proposal({})", msg.block),
            ProposalOrVote::Vote(msg) => write!(
                f,
                "Vote({}, {}, {:?}, {})",
                msg.block_id, msg.block_height, msg.decision, msg.signature.public_key
            ),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Needs sync")]
pub struct NeedsSync;
