//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::{debug, info};
use tari_dan_storage::consensus_models::Block;
use tari_epoch_manager::EpochManagerReader;

use super::{HotStuffError, HotstuffConfig};
use crate::{
    messages::{HotstuffMessage, ProposalMessage},
    traits::{ConsensusSpec, OutboundMessaging},
};

#[derive(Clone)]
pub struct Proposer<TConsensusSpec: ConsensusSpec> {
    config: HotstuffConfig,
    epoch_manager: TConsensusSpec::EpochManager,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
}

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_propose_foreignly";

impl<TConsensusSpec> Proposer<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        config: HotstuffConfig,
        epoch_manager: TConsensusSpec::EpochManager,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
    ) -> Self {
        Self {
            config,
            epoch_manager,
            outbound_messaging,
        }
    }

    pub async fn broadcast_foreign_proposal_if_required(&mut self, block: Block) -> Result<(), HotStuffError> {
        let num_committees = self.epoch_manager.get_num_committees(block.epoch()).await?;

        let validator = self.epoch_manager.get_our_validator_node(block.epoch()).await?;
        let local_shard_group = validator
            .shard_key
            .to_shard_group(self.config.num_preshards, num_committees);
        let non_local_shard_groups = block
            .commands()
            .iter()
            .filter_map(|c| c.local_prepared())
            .flat_map(|p| p.evidence.substate_addresses_iter())
            .map(|addr| addr.to_shard_group(self.config.num_preshards, num_committees))
            .filter(|shard_group| local_shard_group != *shard_group)
            .collect::<HashSet<_>>();
        if non_local_shard_groups.is_empty() {
            return Ok(());
        }
        info!(
            target: LOG_TARGET,
            "🌿 PROPOSING new locked block {} to {} foreign shard groups. justify: {} ({}), parent: {}",
            block,
            non_local_shard_groups.len(),
            block.justify().block_id(),
            block.justify().block_height(),
            block.parent()
        );
        debug!(
            target: LOG_TARGET,
            "non_local_shards : [{}]",
            non_local_shard_groups.iter().map(|s|s.to_string()).collect::<Vec<_>>().join(","),
        );

        let mut addresses = HashSet::new();
        // TODO(perf): fetch only applicable committee addresses
        let mut committees = self.epoch_manager.get_committees(block.epoch()).await?;
        for shard_group in non_local_shard_groups {
            addresses.extend(
                committees
                    .remove(&shard_group)
                    .into_iter()
                    .flat_map(|c| c.into_iter().map(|(addr, _)| addr)),
            );
        }
        info!(
            target: LOG_TARGET,
            "🌿 FOREIGN PROPOSE: Broadcasting locked block {} to {} foreign committees.",
            block,
            addresses.len(),
        );
        self.outbound_messaging
            .multicast(&addresses, HotstuffMessage::ForeignProposal(ProposalMessage { block }))
            .await?;
        Ok(())
    }
}
