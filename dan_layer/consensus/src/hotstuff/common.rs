//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::{committee::Committee, Epoch, NodeAddressable, NodeHeight};
use tari_dan_storage::consensus_models::{Block, QuorumCertificate, QuorumDecision};

use crate::{messages::HotstuffMessage, traits::LeaderStrategy};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::common";

/// The value that fees are divided by to determine the amount of fees to burn. 0 means no fees are burned.
/// This is a placeholder for the fee exhaust consensus constant so that we know where it's used later.
/// TODO: exhaust > 0
pub const EXHAUST_DIVISOR: u64 = 0;

// To avoid clippy::type_complexity
pub(super) type CommitteeAndMessage<TAddr> = (Committee<TAddr>, HotstuffMessage<TAddr>);

pub fn calculate_dummy_blocks<TAddr: NodeAddressable, TLeaderStrategy: LeaderStrategy<TAddr>>(
    epoch: Epoch,
    high_qc: &QuorumCertificate<TAddr>,
    new_height: NodeHeight,
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
) -> Vec<Block<TAddr>> {
    let mut parent_block = high_qc.as_leaf_block();
    let mut current_height = high_qc.block_height() + NodeHeight(1);
    if current_height > new_height {
        warn!(
            target: LOG_TARGET,
            "BUG: 🍼 no dummy blocks to calculate. current height {} is greater than new height {}",
            current_height,
            new_height,
        );
        return Vec::new();
    }

    debug!(
        target: LOG_TARGET,
        "🍼 calculating dummy blocks from {} to {}",
        current_height,
        new_height,
    );
    let num_blocks = new_height.saturating_sub(current_height).as_u64() as usize;
    let mut blocks = Vec::with_capacity(num_blocks);
    loop {
        let leader = leader_strategy.get_leader(local_committee, current_height);
        let dummy_block = Block::dummy_block(
            *parent_block.block_id(),
            leader.clone(),
            current_height,
            high_qc.clone(),
            epoch,
        );
        debug!(
            target: LOG_TARGET,
            "🍼 new dummy block: {}",
            dummy_block,
        );
        parent_block = dummy_block.as_leaf_block();
        blocks.push(dummy_block);

        if current_height == new_height {
            break;
        }
        current_height += NodeHeight(1);
    }

    blocks
}

#[derive(Debug)]
pub struct BlockDecision(bool);

impl BlockDecision {
    pub fn vote_accept() -> Self {
        Self(true)
    }

    pub fn is_accept(&self) -> bool {
        self.0
    }

    pub fn as_quorum_decision(&self) -> Option<QuorumDecision> {
        if self.0 {
            Some(QuorumDecision::Accept)
        } else {
            None
        }
    }

    pub fn dont_vote(&mut self) {
        self.0 = false;
    }
}
