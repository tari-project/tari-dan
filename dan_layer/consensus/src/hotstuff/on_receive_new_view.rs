//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::*;
use tari_common::configuration::Network;
use tari_dan_common_types::{committee::CommitteeInfo, optional::Optional, Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{Block, BlockId, LeafBlock, LockedBlock, QuorumCertificate},
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use super::vote_collector::VoteCollector;
use crate::{
    hotstuff::{common::calculate_last_dummy_block, error::HotStuffError, pacemaker_handle::PaceMakerHandle},
    messages::NewViewMessage,
    tracing::TraceTimer,
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
    vote_collector: VoteCollector<TConsensusSpec>,
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
        vote_receiver: VoteCollector<TConsensusSpec>,
    ) -> Self {
        Self {
            network,
            store,
            leader_strategy,
            epoch_manager,
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
        from: TConsensusSpec::Addr,
        message: NewViewMessage,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "OnReceiveNewView");
        let NewViewMessage {
            high_qc,
            new_height,
            last_vote,
        } = message;
        let epoch = high_qc.epoch();
        debug!(
            target: LOG_TARGET,
            "🌟 Received NEWVIEW {} for qc {} from {}",
            new_height,
            high_qc,
            from
        );

        if epoch != current_epoch {
            warn!(target: LOG_TARGET, "❌ Ignoring NEWVIEW for epoch {} because the epoch is not the current epoch", epoch);
            return Ok(());
        }

        // TODO: This prevents syncing the blocks from previous epoch.
        // if !self.epoch_manager.is_validator_in_local_committee(&from, epoch).await? {
        //     return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
        //         epoch,
        //         sender: from.to_string(),
        //         context: format!("Received NEWVIEW from {}", from),
        //     });
        // }

        // We can never accept NEWVIEWS for heights that are lower than the locked block height
        self.store.with_read_tx(|tx| {
            let locked = LockedBlock::get(tx, epoch)?;
            if new_height < locked.height() {
                warn!(target: LOG_TARGET, "❌ Ignoring NEWVIEW for height less than the locked block, locked block: {} new height: {}", locked, new_height);
                return Ok(());
            }

            self.validate_qc(&high_qc)?;

            if !Block::record_exists(tx, high_qc.block_id())? {
                // Sync if we do not have the block for this valid QC
                let local_height = LeafBlock::get(tx, epoch)
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

        let local_committee = self.epoch_manager.get_local_committee(epoch).await?;
        let leader = self
            .leader_strategy
            .get_leader_for_next_block(&local_committee, new_height);
        let our_node = self.epoch_manager.get_our_validator_node(epoch).await?;

        if *leader != our_node.address {
            warn!(target: LOG_TARGET, "❌ New View failed, leader is {} at height:{}", leader, new_height);
            return Ok(());
        }

        // Are nodes requesting to create more than the minimum number of dummy blocks?
        let height_diff = high_qc.block_height().saturating_sub(new_height).as_u64();
        if height_diff > local_committee.len() as u64 {
            warn!(
                target: LOG_TARGET,
                "❌ Validator {from} sent NEWVIEW that attempts to create a larger than necessary number of dummy blocks. Expected requested {} < local committee size {}",
                height_diff,
                local_committee.len()
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
            let high_qc = high_qc.update_high_qc(tx)?;
            high_qc.get_quorum_certificate(&**tx)
        })?;

        let threshold = local_committee_info.quorum_threshold() as usize;

        info!(
            target: LOG_TARGET,
            "🌟 Received NEWVIEW (QUORUM: {}/{}) {} (QC: {})",
            newview_count,
            threshold,
            new_height,
            latest_high_qc,
        );
        // Once we have received enough (quorum) NEWVIEWS, we can create the dummy block(s) and propose the next block.
        // Any subsequent NEWVIEWs for this height/view are ignored.
        if newview_count == threshold {
            info!(target: LOG_TARGET, "🌟✅ NEWVIEW height {} (high_qc: {}) has reached quorum ({}/{})", new_height, latest_high_qc.as_high_qc(), newview_count, threshold);
            self.pacemaker
                .update_view(epoch, new_height, high_qc.block_height())
                .await?;
            if latest_high_qc.block_height() + NodeHeight(1) > new_height {
                // CASE: the votes received from NEWVIEWS created a new high QC, so there are no dummy blocks to create
                // We can force beat with our current leaf and the justified block is the parent.
                self.pacemaker.force_beat_current_leaf();
                return Ok(());
            }

            let high_qc_block = self.store.with_read_tx(|tx| latest_high_qc.get_block(tx))?;
            // Determine how many missing blocks we must fill without actually creating them.
            // This node, as well as all other replicas, will create the blocks in on_receive_proposal.
            let last_dummy_block = calculate_last_dummy_block(
                self.network,
                epoch,
                latest_high_qc.shard_group(),
                &latest_high_qc,
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

    fn validate_qc(&self, _qc: &QuorumCertificate) -> Result<(), HotStuffError> {
        // TODO
        Ok(())
    }
}
