//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{BTreeSet, HashSet};

use log::{debug, info};
use tari_dan_common_types::shard::Shard;
use tari_dan_storage::{
    consensus_models::{Block, Command, ExecutedTransaction},
    StateStore,
    StateStoreReadTransaction,
};
use tari_epoch_manager::EpochManagerReader;

use super::HotStuffError;
use crate::{
    messages::{HotstuffMessage, ProposalMessage},
    traits::{ConsensusSpec, OutboundMessaging},
};

#[derive(Clone)]
pub struct Proposer<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
}

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_propose_foreignly";

impl<TConsensusSpec> Proposer<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            outbound_messaging,
        }
    }

    pub async fn broadcast_foreign_proposal_if_required(&mut self, block: Block) -> Result<(), HotStuffError> {
        let num_committees = self.epoch_manager.get_num_committees(block.epoch()).await?;

        let validator = self.epoch_manager.get_our_validator_node(block.epoch()).await?;
        let local_shard = validator.shard_key.to_committee_shard(num_committees);
        let non_local_shards = self
            .store
            .with_read_tx(|tx| get_non_local_shards(tx, &block, num_committees, local_shard))?;
        if non_local_shards.is_empty() {
            return Ok(());
        }
        info!(
            target: LOG_TARGET,
            "🌿 PROPOSING foreignly new locked block {} to {} foreign shards. justify: {} ({}), parent: {}",
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
            "🌿 Broadcasting new locked block {} to {} foreign committees.",
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

pub fn get_non_local_shards<TTx: StateStoreReadTransaction>(
    tx: &mut TTx,
    block: &Block,
    num_committees: u32,
    local_shard: Shard,
) -> Result<HashSet<Shard>, HotStuffError> {
    get_non_local_shards_from_commands(tx, block.commands(), num_committees, local_shard)
}

pub fn get_non_local_shards_from_commands<TTx: StateStoreReadTransaction>(
    tx: &mut TTx,
    commands: &BTreeSet<Command>,
    num_committees: u32,
    local_shard: Shard,
) -> Result<HashSet<Shard>, HotStuffError> {
    let prepared_iter = commands.iter().filter_map(|cmd| cmd.local_prepared()).map(|t| &t.id);
    let prepared_txs = ExecutedTransaction::get_involved_shards(tx, prepared_iter)?;
    let non_local_shards = prepared_txs
        .into_iter()
        .flat_map(|(_, addresses)| addresses)
        .map(|address| address.to_committee_shard(num_committees))
        .filter(|shard| *shard != local_shard)
        .collect();
    Ok(non_local_shards)
}
