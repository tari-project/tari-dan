//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, iter};

use log::*;
use tari_dan_common_types::{committee::Committee, optional::Optional, Epoch, NodeHeight, ShardId};
use tari_dan_storage::{
    consensus_models::{
        Block,
        HighQc,
        LeafBlock,
        NewTransactionPool,
        PrecommitTransactionPool,
        PrepareTransactionPool,
        QuorumCertificate,
    },
    StateStore,
    StateStoreWriteTransaction,
};
use tokio::sync::mpsc;

use crate::{
    hotstuff::error::HotStuffError,
    messages::{HotstuffMessage, ProposalMessage},
    traits::{ConsensusSpec, EpochManager},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_propose";

pub struct OnPropose<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    tx_broadcast: mpsc::Sender<(Committee<TConsensusSpec::Addr>, HotstuffMessage)>,
}

impl<TConsensusSpec> OnPropose<TConsensusSpec>
where
    TConsensusSpec: ConsensusSpec,
    HotStuffError: From<<TConsensusSpec::EpochManager as EpochManager>::Error>,
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        tx_broadcast: mpsc::Sender<(Committee<TConsensusSpec::Addr>, HotstuffMessage)>,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            tx_broadcast,
        }
    }

    pub async fn handle(
        &self,
        epoch: Epoch,
        local_committee: Committee<TConsensusSpec::Addr>,
        leaf_block: LeafBlock,
    ) -> Result<(), HotStuffError> {
        let validator_id = self.epoch_manager.get_our_validator_shard(epoch).await?;

        // The scope here is due to a shortcoming of rust. The tx is dropped at tx.commit() but it still complains that
        // the non-Send tx could be used after the await point, which is not possible.
        let involved_shards;
        let next_block;
        {
            let mut tx = self.store.create_write_tx()?;
            let high_qc = HighQc::get(&mut *tx, epoch)?;
            let high_qc = high_qc.get_quorum_certificate(&mut *tx)?;

            let parent_block = leaf_block.get_block(&mut *tx)?;

            // Edge case: on_beat is called again before the leaf has changed - so check that we haven't already
            // proposed for this leaf
            if let Some(child) = parent_block.get_child(&mut *tx).optional()? {
                debug!(
                    target: LOG_TARGET,
                    "🌿 Already proposed block for current leaf {} ({}) - child {} ({})",
                    parent_block.id(),
                    parent_block.height(),
                    child.id(),
                    child.height(),
                );
                tx.rollback()?;
                return Ok(());
            }

            next_block = self.build_next_block(&mut *tx, epoch, &parent_block, high_qc, validator_id)?;
            if next_block.exists(&mut *tx)? {
                debug!(
                    target: LOG_TARGET,
                    "🌿 Already proposed block {} ({})",
                    next_block.id(),
                    next_block.height(),
                );
                tx.rollback()?;
                return Ok(());
            }
            involved_shards = next_block.find_involved_shards(&mut *tx)?;
            next_block.insert(&mut tx)?;

            tx.commit()?;
        }

        info!(
            target: LOG_TARGET,
            "🌿 PROPOSING new block {}({}) (justify: {} ({})) -> {}",
            next_block.id(),
            next_block.height(),
            next_block.justify().block_id(),
            next_block.justify().block_height(),
            next_block.parent()
        );

        self.broadcast_proposal(epoch, next_block, involved_shards, local_committee, validator_id)
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
        tx: &mut <TConsensusSpec::StateStore as StateStore>::ReadTransaction<'_>,
        epoch: Epoch,
        parent_block: &Block,
        high_qc: QuorumCertificate,
        proposed_by: ShardId,
    ) -> Result<Block, HotStuffError> {
        // TODO: Configure
        const TARGET_BLOCK_SIZE: usize = 1000;
        let mut remaining_txs = TARGET_BLOCK_SIZE;

        // TODO: Decide if we should lock the txs to the next block somehow
        // Fetch transactions that are ready to go into a block. They will be moved into the correct pool in
        // OnReceiveProposal
        let commit_txs = PrecommitTransactionPool::get_batch(tx, remaining_txs)?;
        remaining_txs -= commit_txs.len();
        let precommit_txs = PrepareTransactionPool::get_batch(tx, remaining_txs)?;
        remaining_txs -= precommit_txs.len();
        let prepare_txs = NewTransactionPool::get_batch(tx, remaining_txs)?;

        let next_block = Block::new(
            *parent_block.id(),
            high_qc,
            parent_block.height() + NodeHeight(1),
            epoch,
            0,
            proposed_by,
            prepare_txs,
            precommit_txs,
            commit_txs,
        );

        Ok(next_block)
    }
}
