//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::*;
use tari_dan_storage::{
    consensus_models::{BlockId, QuorumCertificate, ValidatorId},
    StateStore,
};

use crate::{
    hotstuff::{common::update_high_qc, error::HotStuffError},
    messages::NewViewMessage,
    traits::{ConsensusSpec, EpochManager},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_new_view";

pub struct OnReceiveNewViewHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    _leader_strategy: TConsensusSpec::LeaderStrategy,
    epoch_manager: TConsensusSpec::EpochManager,
    newview_message_counts: HashMap<BlockId, HashSet<ValidatorId>>,
}

impl<TConsensusSpec> OnReceiveNewViewHandler<TConsensusSpec>
where
    TConsensusSpec: ConsensusSpec,
    HotStuffError: From<<TConsensusSpec::EpochManager as EpochManager>::Error>,
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        epoch_manager: TConsensusSpec::EpochManager,
    ) -> Self {
        Self {
            store,
            _leader_strategy: leader_strategy,
            epoch_manager,
            newview_message_counts: HashMap::default(),
        }
    }

    pub async fn handle(&mut self, from: ValidatorId, message: NewViewMessage) -> Result<(), HotStuffError> {
        let NewViewMessage { high_qc } = message;
        debug!(
            target: LOG_TARGET,
            "🔥 Receive NEWVIEW for block {}",
            high_qc.block_id(),
        );

        if !self.epoch_manager.is_epoch_active(high_qc.epoch()).await? {
            return Err(HotStuffError::EpochNotActive {
                epoch: high_qc.epoch(),
                context: format!("Received NEWVIEW from {}", from),
            });
        }

        if !self
            .epoch_manager
            .is_validator_in_local_committee(from, high_qc.epoch())
            .await?
        {
            return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
                epoch: high_qc.epoch(),
                sender: from,
                context: format!("Received NEWVIEW from {}", from),
            });
        }

        self.validate_qc(&high_qc)?;

        self.store
            .with_write_tx(|tx| update_high_qc::<TConsensusSpec::StateStore>(tx, from, &high_qc))?;

        // // Take note of unique NEWVIEWs so that we can count them
        let entry = self.newview_message_counts.entry(*high_qc.block_id()).or_default();
        entry.insert(from);

        // self.on_beat()

        Ok(())
    }

    fn validate_qc(&self, _qc: &QuorumCertificate) -> Result<(), HotStuffError> {
        // TODO
        Ok(())
    }

    // async fn on_beat(&mut self, high_qc: HighQc) -> Result<(), HotStuffError> {
    //     let committee = self.epoch_manager.get_committee(high_qc.epoch, shard).await?;
    //     if committee.is_empty() {
    //         return Err(HotStuffError::NoCommitteeForShard { shard, epoch });
    //     }
    //     if self.is_leader(payload_id, shard, &committee)? {
    //         let min_required_new_views = committee.consensus_threshold();
    //         let num_new_views = self.get_newview_count_for(shard, payload_id);
    //         if num_new_views >= min_required_new_views {
    //             self.newview_message_counts.remove(&(shard, payload_id));
    //             self.leader_on_propose(shard, payload_id).await?;
    //         } else {
    //             info!(
    //                 target: LOG_TARGET,
    //                 "🔥 Waiting for more NEWVIEW messages ({}/{}) for shard {}, payload {}",
    //                 num_new_views,
    //                 min_required_new_views,
    //                 shard,
    //                 payload_id
    //             );
    //         }
    //     }
    //     Ok(())
    // }
}
