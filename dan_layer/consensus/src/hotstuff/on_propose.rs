//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::BTreeSet, num::NonZeroU64};

use log::*;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    optional::Optional,
    Epoch,
    NodeHeight,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        Command,
        HighQc,
        LastProposed,
        LeafBlock,
        QuorumCertificate,
        TransactionPool,
        TransactionPoolStage,
    },
    StateStore,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::EpochManagerReader;
use tokio::sync::mpsc;

use super::common::CommitteeAndMessage;
use crate::{
    hotstuff::{common::EXHAUST_DIVISOR, error::HotStuffError},
    messages::{HotstuffMessage, ProposalMessage},
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_propose_locally";

pub struct OnPropose<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    tx_broadcast: mpsc::Sender<CommitteeAndMessage<TConsensusSpec::Addr>>,
}

impl<TConsensusSpec> OnPropose<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        tx_broadcast: mpsc::Sender<CommitteeAndMessage<TConsensusSpec::Addr>>,
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
        is_newview_propose: bool,
    ) -> Result<(), HotStuffError> {
        if let Some(last_proposed) = self.store.with_read_tx(|tx| LastProposed::get(tx)).optional()? {
            if last_proposed.height > leaf_block.height {
                // is_newview_propose means that a NEWVIEW has reached quorum and nodes are expecting us to propose.
                // Re-broadcast the previous proposal
                if is_newview_propose {
                    if let Some(next_block) = self.store.with_read_tx(|tx| last_proposed.get_block(tx)).optional()? {
                        info!(
                            target: LOG_TARGET,
                            "🌿 RE-BROADCASTING locally block {}({}) to {} validators. {} command(s), justify: {} ({}), parent: {}",
                            next_block.id(),
                            next_block.height(),
                            local_committee.len(),
                            next_block.commands().len(),
                            next_block.justify().block_id(),
                            next_block.justify().block_height(),
                            next_block.parent(),
                        );
                        self.broadcast_proposal_locally(next_block, local_committee).await?;
                        return Ok(());
                    }
                }

                info!(
                    target: LOG_TARGET,
                    "⤵️ SKIPPING propose for leaf {} because we already proposed block {}",
                    leaf_block,
                    last_proposed,
                );

                return Ok(());
            }
        }

        let validator = self.epoch_manager.get_our_validator_node(epoch).await?;
        let local_committee_shard = self.epoch_manager.get_local_committee_shard(epoch).await?;
        // The scope here is due to a shortcoming of rust. The tx is dropped at tx.commit() but it still complains that
        // the non-Send tx could be used after the await point, which is not possible.
        let next_block;
        {
            let mut tx = self.store.create_write_tx()?;
            let high_qc = HighQc::get(&mut *tx)?;
            let high_qc = high_qc.get_quorum_certificate(&mut *tx)?;

            next_block = self.build_next_block(
                &mut tx,
                epoch,
                &leaf_block,
                high_qc,
                validator.address,
                &local_committee_shard,
                // TODO: This just avoids issues with proposed transactions causing leader failures. Not sure if this
                //       is a good idea.
                is_newview_propose,
            )?;

            next_block.as_last_proposed().set(&mut tx)?;

            // Get involved shards for all LocalPrepared commands in the block.
            // This allows us to broadcast the proposal only to the relevant committees that would be interested in the
            // LocalPrepared.
            // TODO: we should never broadcast to foreign shards here. The soonest we can broadcast is once we have
            //       locked the block
            tx.commit()?;
        }

        info!(
            target: LOG_TARGET,
            "🌿 PROPOSING locally new block {} to {} validators. justify: {} ({}), parent: {}",
            next_block,
            local_committee.len(),
            next_block.justify().block_id(),
            next_block.justify().block_height(),
            next_block.parent()
        );

        self.broadcast_proposal_locally(next_block, local_committee).await?;

        Ok(())
    }

    pub async fn broadcast_proposal_locally(
        &self,
        next_block: Block<TConsensusSpec::Addr>,
        local_committee: Committee<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🌿 Broadcasting locally proposal {} to {} local committees",
            next_block,
            local_committee.len(),
        );

        // Broadcast to local and foreign committees
        self.tx_broadcast
            .send((
                local_committee,
                HotstuffMessage::Proposal(ProposalMessage {
                    block: next_block.clone(),
                }),
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
        parent_block: &LeafBlock,
        high_qc: QuorumCertificate<TConsensusSpec::Addr>,
        proposed_by: <TConsensusSpec::EpochManager as EpochManagerReader>::Addr,
        local_committee_shard: &CommitteeShard,
        empty_block: bool,
    ) -> Result<Block<TConsensusSpec::Addr>, HotStuffError> {
        // TODO: Configure
        const TARGET_BLOCK_SIZE: usize = 1000;
        let batch = if empty_block {
            vec![]
        } else {
            self.transaction_pool.get_batch_for_next_block(tx, TARGET_BLOCK_SIZE)?
        };

        let mut total_leader_fee = 0;
        let commands = batch
            .into_iter()
            .map(|t| match t.current_stage() {
                // If the transaction is New, propose to Prepare it
                TransactionPoolStage::New => Ok(Command::Prepare(t.get_local_transaction_atom())),
                // The transaction is Prepared, this stage is only _ready_ once we know that all local nodes
                // accepted Prepared so we propose LocalPrepared
                TransactionPoolStage::Prepared => Ok(Command::LocalPrepared(t.get_local_transaction_atom())),
                // The transaction is LocalPrepared, meaning that we know that all foreign and local nodes have
                // prepared. We can now propose to Accept it. We also propose the decision change which everyone should
                // agree with if they received the same foreign LocalPrepare.
                TransactionPoolStage::LocalPrepared => {
                    let involved = local_committee_shard.count_distinct_buckets(t.transaction().evidence.shards_iter());
                    let involved = NonZeroU64::new(involved as u64).ok_or_else(|| {
                        HotStuffError::InvariantError(format!(
                            "Number of involved shards is zero for transaction {}",
                            t.transaction_id(),
                        ))
                    })?;
                    let leader_fee = t.calculate_leader_fee(involved, EXHAUST_DIVISOR);
                    total_leader_fee += leader_fee;
                    Ok(Command::Accept(t.get_final_transaction_atom(leader_fee)))
                },
                // Not reachable as there is nothing to propose for these stages. To confirm that all local nodes agreed
                // with the Accept, more (possibly empty) blocks with QCs will be proposed and accepted,
                // otherwise the Accept block will not be committed.
                TransactionPoolStage::AllPrepared | TransactionPoolStage::SomePrepared => {
                    unreachable!(
                        "It is invalid for TransactionPoolStage::{} to be ready to propose",
                        t.current_stage()
                    )
                },
            })
            .collect::<Result<BTreeSet<_>, HotStuffError>>()?;

        debug!(
            target: LOG_TARGET,
            "command(s) for next block: [{}]",
            commands.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(",")
        );

        let next_block = Block::new(
            *parent_block.block_id(),
            high_qc,
            parent_block.height() + NodeHeight(1),
            epoch,
            proposed_by,
            commands,
            total_leader_fee,
        );

        Ok(next_block)
    }
}
