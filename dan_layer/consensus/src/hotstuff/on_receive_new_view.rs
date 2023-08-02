//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::*;
use tari_dan_common_types::NodeHeight;
use tari_dan_storage::{
    consensus_models::{Block, BlockId, LeafBlock, QuorumCertificate},
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
    on_beat: PaceMakerHandle,
}

impl<TConsensusSpec> OnReceiveNewViewHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        epoch_manager: TConsensusSpec::EpochManager,
        on_beat: PaceMakerHandle,
    ) -> Self {
        Self {
            store,
            leader_strategy,
            epoch_manager,
            newview_message_counts: HashMap::default(),
            on_beat,
        }
    }

    pub async fn handle(&mut self, from: TConsensusSpec::Addr, message: NewViewMessage) -> Result<(), HotStuffError> {
        let NewViewMessage { high_qc, new_height } = message;
        debug!(
            target: LOG_TARGET,
            "🔥 Receive NEWVIEW for qc {} new height {} from {}",
            high_qc.id(),
            new_height,
            from
        );

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
        entry.insert(from);
        let threshold = self
            .epoch_manager
            .get_local_threshold_for_epoch(high_qc.epoch())
            .await?;
        debug!(
        target: LOG_TARGET,
        "🔥 NEWVIEW for block {} has {} votes out of {}",
        high_qc.block_id(),
        entry.len(),
            threshold
        );
        // look at equal to, so that we only propose once
        if entry.len() == threshold {
            debug!(target: LOG_TARGET, "🔥 NEWVIEW for block {} new height {} has reached quorum", high_qc.block_id(), new_height);

            // Determine how many missing blocks we must fill.
            let local_committee = self.epoch_manager.get_local_committee(high_qc.epoch()).await?;
            let our_node = self
                .epoch_manager
                .get_our_validator_node(high_qc.epoch())
                .await?
                .address;

            let mut leaf_block = self.store.with_read_tx(|tx| LeafBlock::get(tx)?.get_block(tx))?;
            // TODO: check if this is an old new view message
            //                 if leaf_block.height() > new_height {
            //                     warn!(target: LOG_TARGET, "🔥 New View failed, we have already moved on from this new
            // view. potentially a bad new view? leaf block:{} new height: {}", leaf_block.height(), new_height);
            //                     return
            //                 }
            self.store.with_write_tx(|tx| {
                let mut leader = self.leader_strategy.get_leader_for_next_block(&local_committee,  leaf_block.height());
                debug!(target: LOG_TARGET, "🔥 New View failed leader is {} at height:{}", leader, leaf_block.height() + NodeHeight(1)   );
                while leader != &our_node {
                    if leaf_block.height() > new_height {
                        warn!(target: LOG_TARGET, "🔥 New View failed, leaf block height {} is greater than new height {}", leaf_block.height(), new_height);
                        return Err(HotStuffError::BadNewViewMessage{ expected_height: leaf_block.height(), received_new_height: new_height });
                    }

                    info!(target: LOG_TARGET, "Creating dummy block for leader {}, height: {}", leader, leaf_block.height() + NodeHeight(1));
                    // TODO: replace with actual leader's propose
                    leaf_block = Block::dummy_block(leaf_block.id().clone(), leader.clone(), leaf_block.height() + NodeHeight(1), high_qc.epoch());
                    leaf_block.save(tx)?;
                    leaf_block.as_leaf_block().set(tx)?;
                    leader = self.leader_strategy.get_leader_for_next_block(&local_committee, leaf_block.height());
                }
                Ok::<(), HotStuffError>(())
            })?;

            self.on_beat.beat().await?;
        }

        Ok(())
    }

    fn validate_qc(&self, _qc: &QuorumCertificate) -> Result<(), HotStuffError> {
        // TODO
        Ok(())
    }
}
