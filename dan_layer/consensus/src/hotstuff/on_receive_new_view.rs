//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::*;
use tari_dan_common_types::NodeHeight;
use tari_dan_storage::{
    consensus_models::{Block, BlockId, LockedBlock, QuorumCertificate},
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{common::update_high_qc, error::HotStuffError, pacemaker_handle::PaceMakerHandle},
    messages::NewViewMessage,
    traits::{ConsensusSpec, LeaderStrategy},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_new_view";

pub struct OnReceiveNewViewHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    epoch_manager: TConsensusSpec::EpochManager,
    newview_message_counts: HashMap<BlockId, HashMap<NodeHeight, HashSet<TConsensusSpec::Addr>>>,
    pacemaker: PaceMakerHandle,
}

impl<TConsensusSpec> OnReceiveNewViewHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        epoch_manager: TConsensusSpec::EpochManager,
        pacemaker: PaceMakerHandle,
    ) -> Self {
        Self {
            store,
            leader_strategy,
            epoch_manager,
            newview_message_counts: HashMap::default(),
            pacemaker,
        }
    }

    pub async fn handle(
        &mut self,
        from: TConsensusSpec::Addr,
        message: NewViewMessage<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        let NewViewMessage { high_qc, new_height } = message;
        debug!(
            target: LOG_TARGET,
            "🔥 Received NEWVIEW for qc {} new height {} from {}",
            high_qc.id(),
            new_height,
            from
        );

        // We can never accept NEWVIEWS for heights that are lower than the locked block height
        let locked = self.store.with_read_tx(|tx| LockedBlock::get(tx))?;
        if new_height <= locked.height() {
            warn!(target: LOG_TARGET, "❌ Ignoring NEWVIEW for height less than equal to locked block, locked block: {} new height: {}", locked.height(), new_height);
            return Ok(());
        }

        if !self
            .epoch_manager
            .is_validator_in_local_committee(&from, high_qc.epoch())
            .await?
        {
            return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
                epoch: high_qc.epoch(),
                sender: from.to_string(),
                context: format!("Received NEWVIEW from {}", from),
            });
        }

        self.validate_qc(&high_qc)?;

        self.store.with_write_tx(|tx| update_high_qc(tx, &high_qc))?;

        // Take note of unique NEWVIEWs so that we can count them
        let entry = self
            .newview_message_counts
            .entry(*high_qc.block_id())
            .or_default()
            .entry(new_height)
            .or_default();
        entry.insert(from.clone());
        let threshold = self
            .epoch_manager
            .get_local_threshold_for_epoch(high_qc.epoch())
            .await?;
        info!(
            target: LOG_TARGET,
            "🔥 NEWVIEW for block {} {} has {} votes out of {}",
            high_qc.block_height(),
            high_qc.block_id(),
            entry.len(),
            threshold
        );
        // look at equal to, so that we only propose once
        if entry.len() == threshold {
            info!(target: LOG_TARGET, "🔥 NEWVIEW for block {} (high_qc: {}) has reached quorum", new_height, high_qc.as_high_qc());

            // Determine how many missing blocks we must fill.
            let local_committee = self.epoch_manager.get_local_committee(high_qc.epoch()).await?;

            self.store.with_write_tx(|tx| {
                let next_height = high_qc.block_height() + NodeHeight(1);
                let leader = self.leader_strategy.get_leader(&local_committee, next_height);

                let dummy_block = Block::dummy_block(*high_qc.block_id(), leader.clone(), next_height, high_qc);
                debug!(target: LOG_TARGET, "🍼 DUMMY BLOCK: {}. Leader: {}", dummy_block, leader);
                // If we sent a new view message for this block, we already created this block and save will be a no-op
                dummy_block.save(tx)?;
                // However, we'll still set it as a leaf block so that we propose a block on top of the dummy block
                dummy_block.as_leaf_block().set(tx)?;

                Ok::<(), HotStuffError>(())
            })?;

            self.pacemaker.force_beat().await?;
        }

        Ok(())
    }

    fn validate_qc(&self, _qc: &QuorumCertificate<TConsensusSpec::Addr>) -> Result<(), HotStuffError> {
        // TODO
        Ok(())
    }
}
