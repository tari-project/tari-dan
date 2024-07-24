//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::{debug, info};
use tari_dan_storage::consensus_models::Block;
use tari_epoch_manager::EpochManagerReader;

use super::HotStuffError;
use crate::{
    messages::{HotstuffMessage, ProposalMessage},
    traits::{ConsensusSpec, OutboundMessaging},
};

#[derive(Clone)]
pub struct Proposer<TConsensusSpec: ConsensusSpec> {
    epoch_manager: TConsensusSpec::EpochManager,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
}

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_propose_foreignly";

impl<TConsensusSpec> Proposer<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        epoch_manager: TConsensusSpec::EpochManager,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
    ) -> Self {
        Self {
            epoch_manager,
            outbound_messaging,
        }
    }

    pub async fn broadcast_foreign_proposal_if_required(&mut self, block: Block) -> Result<(), HotStuffError> {
        let num_committees = self.epoch_manager.get_num_committees(block.epoch()).await?;

        let validator = self.epoch_manager.get_our_validator_node(block.epoch()).await?;
        let local_shard = validator.shard_key.to_shard(num_committees);
        let non_local_shards = block
            .commands()
            .iter()
            .filter_map(|c| c.local_prepared())
            .flat_map(|p| p.evidence.substate_addresses_iter())
            .map(|addr| addr.to_shard(num_committees))
            .filter(|shard| *shard != local_shard)
            .collect::<HashSet<_>>();
        if non_local_shards.is_empty() {
            return Ok(());
        }
        info!(
            target: LOG_TARGET,
            "🌿 PROPOSING new locked block {} to {} foreign shards. justify: {} ({}), parent: {}",
            block,
            non_local_shards.len(),
            block.justify().block_id(),
            block.justify().block_height(),
            block.parent()
        );
        debug!(
            target: LOG_TARGET,
            "non_local_shards : [{}]",
            non_local_shards.iter().map(|s|s.to_string()).collect::<Vec<_>>().join(","),
        );
        let non_local_committees = self
            .epoch_manager
            .get_committees_by_shards(block.epoch(), non_local_shards)
            .await?;
        info!(
            target: LOG_TARGET,
            "🌿 FOREIGN PROPOSE: Broadcasting locked block {} to {} foreign committees.",
            block,
            non_local_committees.len(),
        );
        self.outbound_messaging
            .multicast(
                non_local_committees
                    .values()
                    .flat_map(|c| c.iter().map(|(addr, _)| addr)),
                HotstuffMessage::ForeignProposal(ProposalMessage { block }),
            )
            .await?;
        Ok(())
    }
}
