//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::{committee::CommitteeInfo, optional::Optional, ShardGroup};
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
use tari_transaction::TransactionId;

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
}

impl<TConsensusSpec> OnReceiveForeignProposalHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        pacemaker: PaceMakerHandle,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            transaction_pool,
            pacemaker,
        }
    }

    pub async fn handle(&mut self, from: TConsensusSpec::Addr, message: ProposalMessage) -> Result<(), HotStuffError> {
        let ProposalMessage { block } = message;

        info!(
            target: LOG_TARGET,
            "🧩 Receive FOREIGN PROPOSAL for block {}, parent {}, height {} from {}",
            block.id(),
            block.parent(),
            block.height(),
            from,
        );

        let mut foreign_receive_counter = self
            .store
            .with_read_tx(|tx| ForeignReceiveCounters::get_or_default(tx))?;

        let vn = self.epoch_manager.get_validator_node(block.epoch(), &from).await?;
        let foreign_committee_info = self
            .epoch_manager
            .get_committee_info_for_substate(block.epoch(), vn.shard_key)
            .await?;

        let local_committee_info = self.epoch_manager.get_local_committee_info(block.epoch()).await?;
        if let Err(err) = self.validate_proposed_block(
            &from,
            &block,
            foreign_committee_info.shard_group(),
            local_committee_info.shard_group(),
            &foreign_receive_counter,
        ) {
            warn!(
                target: LOG_TARGET,
                "⚠️ FOREIGN PROPOSAL: Invalid proposal from {}: {}. Ignoring.",
                from,
                err
            );
            // Invalid blocks should not cause the state machine to transition to Error
            return Ok(());
        }

        foreign_receive_counter.increment_group(foreign_committee_info.shard_group());

        let tx_ids = block
            .commands()
            .iter()
            .filter_map(|command| {
                if let Some(tx) = command.local_prepared() {
                    if !foreign_committee_info.includes_any_shard(command.evidence().substate_addresses_iter()) {
                        return None;
                    }
                    // We are interested in the commands that are for us, they will be in local prepared and one of the
                    // evidence shards will be ours
                    Some(tx.id)
                } else {
                    None
                }
            })
            .collect::<Vec<TransactionId>>();

        // The block height was validated earlier, so we can use the height only and not store the hash anymore
        let foreign_proposal = ForeignProposal::new(
            foreign_committee_info.shard_group(),
            *block.id(),
            tx_ids,
            block.base_layer_block_height(),
        );

        if self.store.with_read_tx(|tx| foreign_proposal.exists(tx))? {
            warn!(
                target: LOG_TARGET,
                "❌ FOREIGN PROPOSAL: Already received proposal for block {}",
                block.id(),
            );
            return Ok(());
        }

        self.store.with_write_tx(|tx| {
            foreign_receive_counter.save(tx)?;
            foreign_proposal.upsert(tx)?;
            self.on_receive_foreign_block(tx, &block, &foreign_committee_info, &local_committee_info)
        })?;

        // We could have ready transactions at this point, so if we're the leader for the next block we can propose
        self.pacemaker.beat();

        Ok(())
    }

    fn on_receive_foreign_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        foreign_committee_info: &CommitteeInfo,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let leaf = LeafBlock::get(&**tx)?;
        // We only want to save the QC once if applicable
        let mut is_qc_saved = false;
        let mut command_count = 0usize;

        for cmd in block.commands() {
            let Some(t) = cmd.local_prepared() else {
                continue;
            };

            if !local_committee_info.includes_any_shard(t.evidence.substate_addresses_iter()) {
                continue;
            }
            let Some(mut tx_rec) = self.transaction_pool.get(tx, leaf, &t.id).optional()? else {
                // TODO: request the transaction
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Foreign proposal received for shard applicable transaction {} but this transaction is unknown. TODO: request it.",
                    t.id
                );
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

            command_count += 1;

            let remote_decision = t.decision;
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

            tx_rec.update_remote_data(tx, remote_decision, *block.justify().id(), foreign_committee_info)?;

            // If all shards are complete and we've already received our LocalPrepared, we can set out LocalPrepared
            // transaction as ready to propose ACCEPT. If we have not received the local LocalPrepared, the transition
            // will happen when we receive the local block.
            if tx_rec.current_stage().is_local_prepared() && tx_rec.evidence().all_shards_justified() {
                info!(
                    target: LOG_TARGET,
                    "🧩 FOREIGN PROPOSAL: Transaction is ready for propose ACCEPT({}, {}) Local Stage: {}",
                    tx_rec.transaction_id(),
                    tx_rec.current_decision(),
                    tx_rec.current_stage()
                );

                tx_rec.add_pending_status_update(tx, leaf, TransactionPoolStage::LocalPrepared, true)?;
            }
        }

        info!(
            target: LOG_TARGET,
            "🧩 FOREIGN PROPOSAL: Processed {} commands from foreign block {}",
            command_count,
            block.id()
        );
        if command_count == 0 {
            warn!(
                target: LOG_TARGET,
                "⚠️ FOREIGN PROPOSAL: No commands were applicable for foreign block {}. Ignoring.",
                block.id()
            );
        }

        Ok(())
    }

    fn validate_proposed_block(
        &self,
        from: &TConsensusSpec::Addr,
        candidate_block: &Block,
        _foreign_shard: ShardGroup,
        _local_shard: ShardGroup,
        _foreign_receive_counter: &ForeignReceiveCounters,
    ) -> Result<(), ProposalValidationError> {
        // TODO: ignoring for now because this is currently broken
        // let Some(incoming_count) = candidate_block.get_foreign_counter(&local_shard) else {
        //     debug!(target:LOG_TARGET, "Our bucket {local_shard:?} is missing reliability index in the proposed block
        // {candidate_block:?}");     return Err(ProposalValidationError::MissingForeignCounters {
        //         proposed_by: from.to_string(),
        //         hash: *candidate_block.id(),
        //     });
        // };
        // let current_count = foreign_receive_counter.get_count(&foreign_shard);
        // if current_count + 1 != incoming_count {
        //     debug!(target:LOG_TARGET, "We were expecting the index to be {expected_count}, but the index was
        // {incoming_count}", expected_count = current_count + 1);     return
        // Err(ProposalValidationError::InvalidForeignCounters {         proposed_by: from.to_string(),
        //         hash: *candidate_block.id(),
        //         details: format!(
        //             "Expected foreign receive count to be {} but it was {}",
        //             current_count + 1,
        //             incoming_count
        //         ),
        //     });
        // }
        if candidate_block.is_genesis() {
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
