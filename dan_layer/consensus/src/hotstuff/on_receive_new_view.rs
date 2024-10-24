//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::*;
use tari_dan_common_types::{
    committee::{Committee, CommitteeInfo},
    optional::Optional,
    Epoch,
    NodeHeight,
};
use tari_dan_storage::{
    consensus_models::{Block, BlockId, LeafBlock, QuorumCertificate},
    StateStore,
};

use super::vote_collector::VoteCollector;
use crate::{
    hotstuff::{error::HotStuffError, pacemaker_handle::PaceMakerHandle},
    messages::NewViewMessage,
    tracing::TraceTimer,
    traits::{ConsensusSpec, LeaderStrategy},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_new_view";

pub struct OnReceiveNewViewHandler<TConsensusSpec: ConsensusSpec> {
    local_validator_addr: TConsensusSpec::Addr,
    store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    newview_message_counts: HashMap<(NodeHeight, BlockId), HashSet<TConsensusSpec::Addr>>,
    pacemaker: PaceMakerHandle,
    vote_collector: VoteCollector<TConsensusSpec>,
}

impl<TConsensusSpec> OnReceiveNewViewHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        local_validator_addr: TConsensusSpec::Addr,
        store: TConsensusSpec::StateStore,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        pacemaker: PaceMakerHandle,
        vote_receiver: VoteCollector<TConsensusSpec>,
    ) -> Self {
        Self {
            local_validator_addr,
            store,
            leader_strategy,
            newview_message_counts: HashMap::default(),
            pacemaker,
            vote_collector: vote_receiver,
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
        self.newview_message_counts
            .retain(|(height, _), _| *height >= new_height);
        if self.newview_message_counts.len() <= 10 && self.newview_message_counts.capacity() > 10 {
            self.newview_message_counts.shrink_to_fit();
        }
        let entry = self
            .newview_message_counts
            .entry((new_height, *high_qc.block_id()))
            .or_default();
        entry.insert(from);
        entry.len()
    }

    #[allow(clippy::too_many_lines)]
    pub async fn handle(
        &mut self,
        current_epoch: Epoch,
        current_height: NodeHeight,
        from: TConsensusSpec::Addr,
        message: NewViewMessage,
        local_committee_info: &CommitteeInfo,
        local_committee: &Committee<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "OnReceiveNewView");
        let NewViewMessage {
            high_qc,
            new_height,
            last_vote,
            ..
        } = message;
        info!(
            target: LOG_TARGET,
            "🌟 NEWVIEW from {from} with new height {new_height} with qc {high_qc}",
        );
        if new_height < current_height {
            warn!(target: LOG_TARGET, "❌ Ignoring NEWVIEW for {new_height} less than or equal the current {current_height}.");
            return Ok(());
        }

        self.store.with_read_tx(|tx| {
            self.validate_qc(&high_qc)?;

            if !Block::record_exists(tx, high_qc.block_id())? {
                // Sync if we do not have the block for this valid QC
                let local_height = LeafBlock::get(tx, current_epoch)
                    .optional()?
                    .map(|leaf| leaf.height())
                    .unwrap_or_default();
                return Err(HotStuffError::FallenBehind {
                    local_height,
                    qc_height: high_qc.block_height(),
                });
            }

            Ok(())
        })?;

        // Check if we are the leader for the view after new_height. We'll set our local view height to the new_height
        // if quorum is reached and propose a block at new_height + 1.
        let (leader, _) = self
            .leader_strategy
            .get_leader_for_next_height(local_committee, new_height);

        if *leader != self.local_validator_addr {
            warn!(target: LOG_TARGET, "❌ NEWVIEW failed, leader is {} at {}. Our address is {}", leader, new_height, self.local_validator_addr);
            return Ok(());
        }

        // Are nodes requesting to create more than the minimum number of dummy blocks?
        let height_diff = high_qc.block_height().saturating_sub(new_height).as_u64();
        if height_diff > u64::from(local_committee_info.quorum_threshold()) {
            warn!(
                target: LOG_TARGET,
                "❌ Validator {from} sent NEWVIEW that attempts to create a larger than necessary number of dummy blocks. Expected requested {} < quorum threshold {}",
                height_diff,
                local_committee_info.quorum_threshold()
            );
            return Ok(());
        }

        if let Some(vote) = last_vote {
            debug!(
                target: LOG_TARGET,
                "🔥 Receive VOTE with NEWVIEW for node {} {} from {}", vote.unverified_block_height, vote.block_id, from,
            );
            self.vote_collector
                .check_and_collect_vote(from.clone(), current_epoch, vote, local_committee_info)
                .await?;
        }

        // Take note of unique NEWVIEWs so that we can count them
        let newview_count = self.collect_new_views(from, new_height, &high_qc);

        let latest_high_qc = self.store.with_write_tx(|tx| {
            high_qc.save(tx)?;
            high_qc.update_high_qc(tx)
        })?;

        let threshold = local_committee_info.quorum_threshold() as usize;

        info!(
            target: LOG_TARGET,
            "🌟 Received NEWVIEW (QUORUM: {}/{}) {} with high {}",
            newview_count,
            threshold,
            new_height,
            latest_high_qc,
        );
        // Once we have received enough (quorum) NEWVIEWS, we can create the dummy block(s) and propose the next block.
        // Any subsequent NEWVIEWs for this height/view are ignored.
        if newview_count == threshold {
            info!(target: LOG_TARGET, "🌟✅ NEWVIEW height {} (high_qc: {}) has reached quorum ({}/{})", new_height, latest_high_qc, newview_count, threshold);

            self.pacemaker
                .update_view(current_epoch, new_height, latest_high_qc.block_height())
                .await?;

            self.pacemaker.force_beat(new_height);
        }

        Ok(())
    }

    fn validate_qc(&self, _qc: &QuorumCertificate) -> Result<(), HotStuffError> {
        // TODO
        Ok(())
    }
}
