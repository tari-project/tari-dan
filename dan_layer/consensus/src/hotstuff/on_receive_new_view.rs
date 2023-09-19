//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::*;
use tari_dan_common_types::NodeHeight;
use tari_dan_storage::{
    consensus_models::{BlockId, LockedBlock, QuorumCertificate},
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{
        common::{calculate_dummy_blocks, update_high_qc},
        error::HotStuffError,
        pacemaker_handle::PaceMakerHandle,
    },
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
        let NewViewMessage {
            high_qc,
            new_height,
            epoch,
        } = message;
        debug!(
            target: LOG_TARGET,
            "🔥 Received NEWVIEW for qc {} new height {} from {}",
            high_qc.id(),
            new_height,
            from
        );

        if !self
            .epoch_manager
            .is_local_validator_registered_for_epoch(epoch)
            .await?
        {
            warn!(target: LOG_TARGET, "❌ Ignoring NEWVIEW for epoch {} because the epoch is invalid or we are not registered for that epoch", epoch);
            return Ok(());
        }

        // We can never accept NEWVIEWS for heights that are lower than the locked block height
        let locked = self.store.with_read_tx(|tx| LockedBlock::get(tx))?;
        if new_height <= locked.height() {
            warn!(target: LOG_TARGET, "❌ Ignoring NEWVIEW for height less than equal to locked block, locked block: {} new height: {}", locked.height(), new_height);
            return Ok(());
        }

        if !self.epoch_manager.is_validator_in_local_committee(&from, epoch).await? {
            return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
                epoch,
                sender: from.to_string(),
                context: format!("Received NEWVIEW from {}", from),
            });
        }

        self.validate_qc(&high_qc)?;

        self.store.with_write_tx(|tx| update_high_qc(tx, &high_qc))?;

        let local_committee = self.epoch_manager.get_local_committee(epoch).await?;
        let leader = self
            .leader_strategy
            .get_leader_for_next_block(&local_committee, new_height);
        let our_node = self.epoch_manager.get_our_validator_node(epoch).await?;

        if *leader != our_node.address {
            warn!(target: LOG_TARGET, "❌ New View failed, leader is {} at height:{}", leader, new_height);
            return Err(HotStuffError::NotTheLeader {
                details: format!(
                    "Received NEWVIEW height {} but this not is not the leader for that height",
                    new_height
                ),
            });
        }

        // Are nodes requesting to create more than the minimum number of dummy blocks?
        if high_qc.block_height().saturating_sub(new_height).as_u64() > local_committee.len() as u64 {
            return Err(HotStuffError::BadNewViewMessage {
                details: format!("Validator {from} requested an invalid number of dummy blocks"),
                high_qc_height: high_qc.block_height(),
                received_new_height: new_height,
            });
        }

        // Take note of unique NEWVIEWs so that we can count them
        let entry = self
            .newview_message_counts
            .entry(*high_qc.block_id())
            .or_default()
            .entry(new_height)
            .or_default();
        entry.insert(from.clone());
        let threshold = self.epoch_manager.get_local_threshold_for_epoch(epoch).await?;
        info!(
            target: LOG_TARGET,
            "🌟 Received NEWVIEW for block {} has {} votes out of {}",
            new_height,
            entry.len(),
            threshold
        );
        // Once we have received enough (quorum) NEWVIEWS, we can create the dummy block(s) and propose the next block.
        // Any subsequent NEWVIEWs for this height/view are ignored.
        if entry.len() == threshold {
            info!(target: LOG_TARGET, "🌟✅ NEWVIEW for block {} (high_qc: {}) has reached quorum", new_height, high_qc.as_high_qc());

            // Determine how many missing blocks we must fill without actually creating them.
            // This node, as well as all other replicas, will create the blocks in on_receive_proposal.
            let dummy_blocks =
                calculate_dummy_blocks(epoch, &high_qc, new_height, &self.leader_strategy, &local_committee);
            let parent_block = dummy_blocks
                .last()
                .map(|b| b.as_leaf_block())
                .unwrap_or_else(|| high_qc.as_leaf_block());

            debug!(target: LOG_TARGET, "🍼 dummy leaf block {}", parent_block);
            // Force beat so that a block is proposed even if there are no transactions
            self.pacemaker.force_beat(parent_block).await?;
        }

        Ok(())
    }

    fn validate_qc(&self, _qc: &QuorumCertificate<TConsensusSpec::Addr>) -> Result<(), HotStuffError> {
        // TODO
        Ok(())
    }
}
