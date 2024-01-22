//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::ops::DerefMut;

use log::*;
use tari_dan_common_types::{committee::CommitteeShard, optional::Optional, shard::Shard, NodeHeight};
use tari_dan_storage::{
    consensus_models::{
        Block,
        ForeignProposal,
        ForeignReceiveCounters,
        LeafBlock,
        TransactionPool,
        TransactionPoolStage,
    },
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{error::HotStuffError, pacemaker_handle::PaceMakerHandle, ProposalValidationError},
    messages::ProposalMessage,
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_foreign_proposal";

pub struct OnReceiveForeignProposalHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    pacemaker: PaceMakerHandle,
    foreign_receive_counter: ForeignReceiveCounters,
}

impl<TConsensusSpec> OnReceiveForeignProposalHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        pacemaker: PaceMakerHandle,
        foreign_receive_counter: ForeignReceiveCounters,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            transaction_pool,
            pacemaker,
            foreign_receive_counter,
        }
    }

    pub async fn handle(&mut self, from: TConsensusSpec::Addr, message: ProposalMessage) -> Result<(), HotStuffError> {
        let ProposalMessage { block } = message;

        debug!(
            target: LOG_TARGET,
            "🔥 Receive FOREIGN PROPOSAL for block {}, parent {}, height {} from {}",
            block.id(),
            block.parent(),
            block.height(),
            from,
        );

        let vn = self.epoch_manager.get_validator_node(block.epoch(), &from).await?;
        let committee_shard = self
            .epoch_manager
            .get_committee_shard(block.epoch(), vn.shard_key)
            .await?;
        let foreign_proposal = ForeignProposal::new(committee_shard.shard(), *block.id());
        if self
            .store
            .with_read_tx(|tx| ForeignProposal::exists(tx, &foreign_proposal))?
        {
            warn!(
                target: LOG_TARGET,
                "🔥 FOREIGN PROPOSAL: Already received proposal for block {}",
                block.id(),
            );
            return Ok(());
        }

        let local_shard = self.epoch_manager.get_local_committee_shard(block.epoch()).await?;
        self.validate_proposed_block(&from, &block, committee_shard.shard(), local_shard.shard())?;
        // Is this ok? Can foreign node send invalid block that should still increment the counter?
        self.foreign_receive_counter.increment(&committee_shard.shard());
        self.store.with_write_tx(|tx| {
            self.foreign_receive_counter.save(tx)?;
            foreign_proposal.upsert(tx)?;
            self.on_receive_foreign_block(tx, &block, &committee_shard)
        })?;

        // We could have ready transactions at this point, so if we're the leader for the next block we can propose
        self.pacemaker.beat();

        Ok(())
    }

    fn on_receive_foreign_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        foreign_committee_shard: &CommitteeShard,
    ) -> Result<(), HotStuffError> {
        let leaf = LeafBlock::get(tx.deref_mut())?;
        // We only want to save the QC once if applicable
        let mut is_qc_saved = false;

        for cmd in block.commands() {
            let Some(t) = cmd.local_prepared() else {
                continue;
            };
            let Some(mut tx_rec) = self.transaction_pool.get(tx, leaf, &t.id).optional()? else {
                continue;
            };

            if tx_rec.current_stage().is_all_prepared() || tx_rec.current_stage().is_some_prepared() {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Foreign proposal received after transaction {} is {}. Ignoring.",
                    tx_rec.transaction_id(), tx_rec.current_stage()
                );
                continue;
            }

            let remote_decision = cmd.decision();
            let local_decision = tx_rec.current_local_decision();
            if remote_decision.is_abort() && local_decision.is_commit() {
                info!(
                    target: LOG_TARGET,
                    "⚠️ Foreign shard ABORT {}. Update overall decision to ABORT. Local stage: {}, Leaf: {}",
                    tx_rec.transaction_id(), tx_rec.current_stage(), leaf
                );
            }

            if !is_qc_saved {
                // Save the QCs if it doesnt exist already, we'll reference the QC in subsequent blocks
                block.justify().save(tx)?;
                is_qc_saved = true;
            }

            tx_rec.update_remote_data(tx, remote_decision, *block.justify().id(), foreign_committee_shard)?;

            // If all shards are complete and we've already received our LocalPrepared, we can set out LocalPrepared
            // transaction as ready to propose ACCEPT. If we have not received the local LocalPrepared, the transition
            // will happen when we receive the local block.
            if tx_rec.current_stage().is_local_prepared() && tx_rec.transaction().evidence.all_shards_complete() {
                info!(
                    target: LOG_TARGET,
                    "🔥 FOREIGN PROPOSAL: Transaction is ready for propose ACCEPT({}, {}) Local Stage: {}",
                    tx_rec.transaction_id(),
                    tx_rec.current_decision(),
                    tx_rec.current_stage()
                );

                tx_rec.add_pending_status_update(tx, leaf, TransactionPoolStage::LocalPrepared, true)?;
            }
        }

        Ok(())
    }

    fn validate_proposed_block(
        &self,
        from: &TConsensusSpec::Addr,
        candidate_block: &Block,
        foreign_bucket: Shard,
        local_bucket: Shard,
    ) -> Result<(), ProposalValidationError> {
        let incoming_index = match candidate_block.get_foreign_index(&local_bucket) {
            Some(i) => *i,
            None => {
                debug!(target:LOG_TARGET, "Our bucket {local_bucket:?} is missing reliability index in the proposed block {candidate_block:?}");
                return Err(ProposalValidationError::MissingForeignCounters {
                    proposed_by: from.to_string(),
                    hash: *candidate_block.id(),
                });
            },
        };
        let current_index = self.foreign_receive_counter.get_index(&foreign_bucket);
        if current_index + 1 != incoming_index {
            debug!(target:LOG_TARGET, "We were expecting the index to be {expected_index}, but the index was
        {incoming_index}", expected_index = current_index + 1);
            return Err(ProposalValidationError::InvalidForeignCounters {
                proposed_by: from.to_string(),
                hash: *candidate_block.id(),
            });
        }
        if candidate_block.height() == NodeHeight::zero() || candidate_block.id().is_genesis() {
            return Err(ProposalValidationError::ProposingGenesisBlock {
                proposed_by: from.to_string(),
                hash: *candidate_block.id(),
            });
        }

        let calculated_hash = candidate_block.calculate_hash().into();
        if calculated_hash != *candidate_block.id() {
            return Err(ProposalValidationError::NodeHashMismatch {
                proposed_by: from.to_string(),
                hash: *candidate_block.id(),
                calculated_hash,
            });
        }

        // TODO: validate justify signatures
        // self.validate_qc(candidate_block.justify(), committee)?;

        Ok(())
    }
}
