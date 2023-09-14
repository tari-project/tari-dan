//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_dan_common_types::{committee::Committee, Epoch, NodeAddressable, NodeHeight};
use tari_dan_storage::{
    consensus_models::{Block, HighQc, QuorumCertificate, QuorumDecision},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

use crate::{messages::HotstuffMessage, traits::LeaderStrategy};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff";

/// The value that fees are divided by to determine the amount of fees to burn. 0 means no fees are burned.
/// This is a placeholder for the fee exhaust consensus constant so that we know where it's used later.
/// TODO: exhaust > 0
pub const EXHAUST_DIVISOR: u64 = 0;

// To avoid clippy::type_complexity
pub(super) type CommitteeAndMessage<TAddr> = (Committee<TAddr>, HotstuffMessage<TAddr>);

pub fn update_high_qc<TTx, TAddr: NodeAddressable>(
    tx: &mut TTx,
    qc: &QuorumCertificate<TAddr>,
) -> Result<(), StorageError>
where
    TTx: StateStoreWriteTransaction<Addr = TAddr> + DerefMut,
    TTx::Target: StateStoreReadTransaction,
{
    let high_qc = HighQc::get(tx.deref_mut())?;
    let high_qc = high_qc.get_quorum_certificate(tx.deref_mut())?;

    if high_qc.block_height() < qc.block_height() {
        debug!(
            target: LOG_TARGET,
            "🔥 UPDATE_HIGH_QC (node: {} {}, previous high QC: {} {})",
            qc.id(),
            qc.block_height(),
            high_qc.block_id(),
            high_qc.block_height(),
        );

        qc.save(tx)?;
        // This will fail if the block doesnt exist
        qc.as_leaf_block().set(tx)?;
        qc.as_high_qc().set(tx)?;
    }

    Ok(())
}

pub fn calculate_dummy_blocks<TAddr: NodeAddressable, TLeaderStrategy: LeaderStrategy<TAddr>>(
    epoch: Epoch,
    high_qc: &QuorumCertificate<TAddr>,
    new_height: NodeHeight,
    leader_strategy: &TLeaderStrategy,
    local_committee: &Committee<TAddr>,
) -> Vec<Block<TAddr>> {
    let mut parent_block = high_qc.as_leaf_block();
    let mut current_height = high_qc.block_height() + NodeHeight(1);
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
