//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::*;
use tari_common::configuration::Network;
use tari_dan_common_types::NodeHeight;
use tari_dan_storage::{
    consensus_models::{Block, BlockId, LeafBlock, LockedBlock, QuorumCertificate},
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use super::vote_receiver::VoteReceiver;
use crate::{
    hotstuff::{common::calculate_last_dummy_block, error::HotStuffError, pacemaker_handle::PaceMakerHandle},
    messages::NewViewMessage,
    validations::new_view_validations::check_new_view_message,
    traits::{ConsensusSpec, LeaderStrategy},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_new_view";

pub struct OnReceiveNewViewHandler<TConsensusSpec: ConsensusSpec> {
    network: Network,
    store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    epoch_manager: TConsensusSpec::EpochManager,
    newview_message_counts: HashMap<(NodeHeight, BlockId), HashSet<TConsensusSpec::Addr>>,
    pacemaker: PaceMakerHandle,
    vote_receiver: VoteReceiver<TConsensusSpec>,
}

impl<TConsensusSpec> OnReceiveNewViewHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        network: Network,
        store: TConsensusSpec::StateStore,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        epoch_manager: TConsensusSpec::EpochManager,
        pacemaker: PaceMakerHandle,
        vote_receiver: VoteReceiver<TConsensusSpec>,
    ) -> Self {
        Self {
            network,
            store,
            leader_strategy,
            epoch_manager,
            newview_message_counts: HashMap::default(),
            pacemaker,
            vote_receiver,
        }
    }

    pub(super) fn clear_new_views(&mut self) {
        self.newview_message_counts.clear();
    }

    fn collect_new_views(
        &mut self,
        from: TConsensusSpec::Addr,
        new_height: NodeHeight,
        high_qc: &QuorumCertificate,
    ) -> usize {
        let entry = self
            .newview_message_counts
            .entry((new_height, *high_qc.block_id()))
            .or_default();
        entry.insert(from);
        entry.len()
    }

    #[allow(clippy::too_many_lines)]
    pub async fn handle(&mut self, from: TConsensusSpec::Addr, message: NewViewMessage) -> Result<(), HotStuffError> {
        let local_committee = self.epoch_manager.get_local_committee(message.epoch).await?;
        let local_committee_shard = self.epoch_manager.get_local_committee_info(message.epoch).await?;
        let locked = self.store.with_read_tx(|tx| LockedBlock::get(tx))?;
        match check_new_view_message::<TConsensusSpec>(
            &message,
            &self.epoch_manager,
            &locked,
            &self.leader_strategy,
            &local_committee,
            &local_committee_shard,
        )
        .await
        {
            Ok(()) => {},
            Err(e) => {
                warn!(target: LOG_TARGET, "❌ Ignoring NEW_VIEW message because it failed validation: {}",  e);
                return Ok(());
            },
        }

        let NewViewMessage {
            high_qc,
            new_height,
            epoch,
            last_vote,
        } = message;
        debug!(
            target: LOG_TARGET,
            "🌟 Received NEWVIEW for qc {} new height {} from {}",
            high_qc,
            new_height,
            from
        );

        // Sync if we do not have the block for this valid QC
        let exists = self
            .store
            .with_read_tx(|tx| Block::record_exists(tx, high_qc.block_id()))?;
        if !exists {
            let leaf = self
                .store
                .with_read_tx(|tx| LeafBlock::get(tx))
                // We need something for the returned error even if this query fails
                .unwrap_or_else(|_| LeafBlock::genesis());
            return Err(HotStuffError::FallenBehind {
                local_height: leaf.height(),
                qc_height: high_qc.block_height(),
                detected_at: "NEWVIEW".to_string(),
            });
        }

        if let Some(vote) = last_vote {
            debug!(
                target: LOG_TARGET,
                "🔥 Receive VOTE with NEWVIEW for node {} from {}", vote.block_id, from,
            );
            self.vote_receiver.handle(from.clone(), vote, false).await?;
        }

        // Take note of unique NEWVIEWs so that we can count them
        let newview_count = self.collect_new_views(from, new_height, &high_qc);

        let high_qc = self.store.with_write_tx(|tx| {
            high_qc.save(tx)?;
            let high_qc = high_qc.update_high_qc(tx)?;
            high_qc.get_quorum_certificate(&**tx)
        })?;

        // if checked_high_qc.block_height() > high_qc.block_height() {
        //     warn!(target: LOG_TARGET, "❌ Ignoring NEWVIEW for because high QC is not higher than previous high QC,
        // given high QC: {} current high QC: {}", high_qc.as_high_qc(), checked_high_qc);     return Ok(());
        // }

        let threshold = self.epoch_manager.get_local_threshold_for_epoch(epoch).await?;

        debug!(
            target: LOG_TARGET,
            "🌟 Received NEWVIEW for height {} (QC: {}) has {} votes out of {}",
            new_height,
            high_qc,
            newview_count,
            threshold,
        );
        // Once we have received enough (quorum) NEWVIEWS, we can create the dummy block(s) and propose the next block.
        // Any subsequent NEWVIEWs for this height/view are ignored.
        if newview_count == threshold {
            debug!(target: LOG_TARGET, "🌟✅ NEWVIEW for block {} (high_qc: {}) has reached quorum ({}/{})", new_height, high_qc.as_high_qc(), newview_count, threshold);

            let high_qc_block = self.store.with_read_tx(|tx| high_qc.get_block(tx))?;
            // Determine how many missing blocks we must fill without actually creating them.
            // This node, as well as all other replicas, will create the blocks in on_receive_proposal.
            let last_dummy_block = calculate_last_dummy_block(
                self.network,
                epoch,
                local_committee_shard.shard(),
                &high_qc,
                *high_qc_block.merkle_root(),
                new_height,
                &self.leader_strategy,
                &local_committee,
                high_qc_block.timestamp(),
                high_qc_block.base_layer_block_height(),
                *high_qc_block.base_layer_block_hash(),
            );
            // Set the last voted block so that we do not vote on other conflicting blocks
            if let Some(last_dummy) = last_dummy_block {
                debug!(target: LOG_TARGET, "🍼 dummy leaf block {}", last_dummy);
                // Force beat so that a block is proposed even if there are no transactions
                self.pacemaker.force_beat(last_dummy);
            } else {
                warn!(target: LOG_TARGET, "❌ No dummy blocks were created for height {}", new_height);
            }
        }

        Ok(())
    }
}
