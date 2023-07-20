//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, iter, ops::DerefMut};

use log::*;
use tari_dan_common_types::{committee::Committee, optional::Optional, Epoch, NodeHeight, ShardId};
use tari_dan_storage::{
    consensus_models::{
        Block,
        ExecutedTransaction,
        HighQc,
        LastProposed,
        LeafBlock,
        QuorumCertificate,
        TransactionPool,
    },
    StateStore,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::EpochManagerReader;
use tokio::sync::mpsc;

use crate::{
    hotstuff::error::HotStuffError,
    messages::{HotstuffMessage, ProposalMessage},
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_propose";

pub struct OnPropose<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    tx_broadcast: mpsc::Sender<(Committee<TConsensusSpec::Addr>, HotstuffMessage)>,
}

impl<TConsensusSpec> OnPropose<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_broadcast: mpsc::Sender<(Committee<TConsensusSpec::Addr>, HotstuffMessage)>,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            transaction_pool,
            tx_broadcast,
        }
    }

    pub async fn handle(
        &self,
        epoch: Epoch,
        local_committee: Committee<TConsensusSpec::Addr>,
        leaf_block: LeafBlock,
    ) -> Result<(), HotStuffError> {
        let last_proposed = self.store.with_read_tx(|tx| LastProposed::get(tx, epoch).optional())?;
        let last_proposed_height = last_proposed.map(|lp| lp.height).unwrap_or(NodeHeight(0));
        if last_proposed_height >= leaf_block.height + NodeHeight(1) {
            info!(
                target: LOG_TARGET,
                "Skipping on_propose for next block because we have already proposed a block at height {}",
                last_proposed_height
            );
            return Ok(());
        }

        let validator = self.epoch_manager.get_our_validator_node(epoch).await?;

        // The scope here is due to a shortcoming of rust. The tx is dropped at tx.commit() but it still complains that
        // the non-Send tx could be used after the await point, which is not possible.
        let involved_foreign_shards;
        let next_block;
        {
            let mut tx = self.store.create_write_tx()?;
            let high_qc = HighQc::get(&mut *tx, epoch)?;
            let high_qc = high_qc.get_quorum_certificate(&mut *tx)?;

            let parent_block = leaf_block.get_block(&mut *tx)?;

            next_block = self.build_next_block(&mut tx, epoch, &parent_block, high_qc, validator.shard_key)?;
            next_block.insert(&mut tx)?;
            next_block.as_last_proposed().set(&mut tx)?;

            // Get involved shards for all LocalPrepared commands in the block.
            // This allows us to broadcast the proposal only to the relevant committees that would be interested in the
            // LocalPrepared.
            let prepared_iter = next_block
                .commands()
                .iter()
                .filter_map(|cmd| cmd.local_prepared())
                .map(|t| &t.id);
            let prepared_txs = ExecutedTransaction::get_many(tx.deref_mut(), prepared_iter)?;
            involved_foreign_shards = prepared_txs
                .iter()
                .flat_map(|tx| tx.transaction().involved_shards_iter().copied())
                .collect::<HashSet<_>>();

            tx.commit()?;
        }

        info!(
            target: LOG_TARGET,
            "🌿 PROPOSING new block {}({}) to {} validators. {} command(s), {} foreign shards, justify: {} ({}), parent: {}",
            next_block.id(),
            next_block.height(),
            local_committee.len(),
            next_block.commands().len(),
            involved_foreign_shards.len(),
            next_block.justify().block_id(),
            next_block.justify().block_height(),
            next_block.parent()
        );

        self.broadcast_proposal(
            epoch,
            next_block,
            involved_foreign_shards,
            local_committee,
            validator.shard_key,
        )
        .await?;

        Ok(())
    }

    async fn broadcast_proposal(
        &self,
        epoch: Epoch,
        next_block: Block,
        involved_shards: HashSet<ShardId>,
        local_committee: Committee<TConsensusSpec::Addr>,
        our_shard_id: ShardId,
    ) -> Result<(), HotStuffError> {
        // Find non-local shard committees to include in the broadcast
        let num_committees = self.epoch_manager.get_num_committees(epoch).await?;
        let local_bucket = our_shard_id.to_committee_bucket(num_committees);
        let non_local_buckets = involved_shards
            .into_iter()
            .map(|shard| shard.to_committee_bucket(num_committees))
            .filter(|bucket| *bucket != local_bucket)
            .collect::<HashSet<_>>();

        let non_local_committees = self
            .epoch_manager
            .get_committees_by_buckets(epoch, non_local_buckets)
            .await?;

        // Broadcast to local and foreign committees
        // TODO: only broadcast to f + 1 foreign committee members. They can gossip the proposal around from there.
        let committee = iter::once(local_committee)
            .chain(non_local_committees.into_values())
            .collect();

        self.tx_broadcast
            .send((
                committee,
                HotstuffMessage::Proposal(ProposalMessage { block: next_block }),
            ))
            .await
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "proposing a new block",
            })?;

        Ok(())
    }

    fn build_next_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        epoch: Epoch,
        parent_block: &Block,
        high_qc: QuorumCertificate,
        proposed_by: ShardId,
    ) -> Result<Block, HotStuffError> {
        // TODO: Configure
        const TARGET_BLOCK_SIZE: usize = 1000;
        let commands = self.transaction_pool.get_batch(tx, TARGET_BLOCK_SIZE)?;

        let next_block = Block::new(
            *parent_block.id(),
            high_qc,
            parent_block.height() + NodeHeight(1),
            epoch,
            0,
            proposed_by,
            commands,
        );

        Ok(next_block)
    }
}
