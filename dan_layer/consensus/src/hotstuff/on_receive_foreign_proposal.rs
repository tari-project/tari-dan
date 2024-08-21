//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::{committee::CommitteeInfo, optional::Optional, ShardGroup};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        BlockPledge,
        Command,
        ForeignProposal,
        ForeignReceiveCounters,
        LeafBlock,
        QuorumCertificate,
        TransactionAtom,
        TransactionPool,
        TransactionPoolRecord,
        TransactionPoolStage,
    },
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::TransactionId;

use crate::{
    hotstuff::{error::HotStuffError, pacemaker_handle::PaceMakerHandle, ProposalValidationError},
    messages::ForeignProposalMessage,
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

    #[allow(clippy::too_many_lines)]
    pub async fn handle(
        &mut self,
        from: TConsensusSpec::Addr,
        message: ForeignProposalMessage,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let ForeignProposalMessage {
            block,
            justify_qc,
            block_pledge,
        } = message;

        // TODO: validate justify_qc
        let mut foreign_receive_counter = self
            .store
            .with_read_tx(|tx| ForeignReceiveCounters::get_or_default(tx))?;

        let vn = self.epoch_manager.get_validator_node(block.epoch(), &from).await?;
        let foreign_committee_info = self
            .epoch_manager
            .get_committee_info_for_substate(block.epoch(), vn.shard_key)
            .await?;

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
                if let Some(tx) = command.local_prepare().or_else(|| command.local_accept()) {
                    if !foreign_committee_info.includes_any_address(command.evidence().substate_addresses_iter()) {
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

        // Justify QC must justify the block
        if justify_qc.block_id() != block.id() {
            warn!(
                target: LOG_TARGET,
                "⚠️ FOREIGN PROPOSAL: Justify QC block id does not match the block id. Justify QC block id: {}, block id: {}",
                justify_qc.block_id(),
                block.id(),
            );
            return Ok(());
        }

        // The block height was validated earlier, so we can use the height only and not store the hash anymore
        let foreign_proposal = ForeignProposal::new(
            foreign_committee_info.shard_group(),
            *block.id(),
            tx_ids,
            block.base_layer_block_height(),
        );

        if self.store.with_read_tx(|tx| foreign_proposal.exists(tx))? {
            // This is expected behaviour, we may receive the same foreign proposal multiple times
            debug!(
                target: LOG_TARGET,
                "FOREIGN PROPOSAL: Already received proposal for block {}",
                block.id(),
            );
            return Ok(());
        }

        info!(
            target: LOG_TARGET,
            "🧩 Receive FOREIGN PROPOSAL for block {}, justify_qc: {} from {}",
            block,
            justify_qc,
            from,
        );

        let result = self.store.with_write_tx(|tx| {
            foreign_receive_counter.save(tx)?;
            foreign_proposal.upsert(tx)?;
            self.on_receive_foreign_block(
                tx,
                &block,
                &justify_qc,
                &foreign_committee_info,
                local_committee_info,
                block_pledge,
            )
        });

        match result {
            Ok(_) => {
                // We could have ready transactions at this point, so if we're the leader for the next block we can
                // propose
                self.pacemaker.beat();
            },
            Err(err) => {
                error!(
                    target: LOG_TARGET,
                    "⚠️ FOREIGN PROPOSAL: Failed to process foreign proposal for block {}: {}",
                    block.id(),
                    err
                );
            },
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn on_receive_foreign_block(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &Block,
        justify_qc: &QuorumCertificate,
        foreign_committee_info: &CommitteeInfo,
        local_committee_info: &CommitteeInfo,
        mut block_pledge: BlockPledge,
    ) -> Result<(), HotStuffError> {
        let local_leaf = LeafBlock::get(&**tx)?;
        // We only want to save the QC once if applicable
        let mut is_qc_saved = false;
        let mut command_count = 0usize;

        for cmd in block.commands() {
            match cmd {
                Command::LocalPrepare(atom) => {
                    if !local_committee_info.includes_any_address(atom.evidence.substate_addresses_iter()) {
                        continue;
                    }

                    let Some(mut tx_rec) = self.transaction_pool.get(tx, local_leaf, &atom.id).optional()? else {
                        // If this happens, it could be a bug in the foreign missing transaction handling
                        warn!(
                            target: LOG_TARGET,
                            "⚠️ NEVER HAPPEN: Foreign proposal received for transaction {} but this transaction is not in the pool.",
                            atom.id
                        );
                        continue;
                    };

                    if tx_rec.current_stage() > TransactionPoolStage::LocalPrepared {
                        warn!(
                            target: LOG_TARGET,
                            "⚠️ Foreign LocalPrepare proposal (shard_group={}, block={}) received for transaction {} but current transaction stage is {}. Ignoring.",
                            foreign_committee_info.shard_group(),
                            block.id(),
                            tx_rec.transaction_id(), tx_rec.current_stage()
                        );
                        continue;
                    }

                    command_count += 1;

                    let remote_decision = atom.decision;
                    let local_decision = tx_rec.current_decision();
                    if remote_decision.is_abort() && local_decision.is_commit() {
                        info!(
                            target: LOG_TARGET,
                            "⚠️ Foreign shard ABORT {}. Update overall decision to ABORT. Local stage: {}, Leaf: {}",
                            tx_rec.transaction_id(), tx_rec.current_stage(), local_leaf
                        );
                    }

                    if !is_qc_saved {
                        // Save the QCs if it doesnt exist already, we'll reference the QC in subsequent blocks
                        block.justify().save(tx)?;
                        justify_qc.save(tx)?;
                        is_qc_saved = true;
                    }

                    // We need to add the justify QC to the evidence because the all prepare block could not include it
                    // yet
                    let mut foreign_evidence = atom.evidence.clone();
                    foreign_evidence.add_qc_evidence(foreign_committee_info, *justify_qc.id());

                    // Update the transaction record with any new information provided by this foreign block
                    tx_rec.update_remote_data(
                        tx,
                        remote_decision,
                        *justify_qc.id(),
                        foreign_committee_info,
                        foreign_evidence,
                    )?;

                    self.validate_and_add_pledges(
                        tx,
                        &tx_rec,
                        block.id(),
                        atom,
                        &mut block_pledge,
                        foreign_committee_info,
                    )?;

                    if tx_rec.current_stage().is_new() {
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalPrepare) Transaction is ready for Prepare({}, {}) Local Stage: {}",
                            tx_rec.transaction_id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );
                        // If the transaction is New, we're waiting for all foreign pledges. Propose transaction once we
                        // have them.

                        // CASE: One foreign SG is involved in all inputs and executed the transaction, local SG is
                        // involved in the outputs
                        let transaction = tx_rec.get_transaction(&**tx)?;
                        let is_ready = local_committee_info.includes_substate_id(&transaction.to_receipt_id().into()) ||
                            transaction.has_any_local_inputs(local_committee_info) ||
                            transaction.has_all_foreign_input_pledges(&**tx, local_committee_info)?;

                        if is_ready {
                            info!(
                                target: LOG_TARGET,
                                "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalPrepare) Transaction is ready for Prepare({}, {}) Local Stage: {}",
                                tx_rec.transaction_id(),
                                tx_rec.current_decision(),
                                tx_rec.current_stage()
                            );
                            tx_rec.add_pending_status_update(tx, local_leaf, TransactionPoolStage::New, true)?;
                        } else {
                            info!(
                                target: LOG_TARGET,
                                "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalPrepare) Transaction is NOT ready for Prepare({}, {}) Local Stage: {}",
                                tx_rec.transaction_id(),
                                tx_rec.current_decision(),
                                tx_rec.current_stage()
                            );
                        }
                    } else if tx_rec.current_stage().is_local_prepared() &&
                        tx_rec.evidence().all_input_addresses_justified()
                    {
                        // If all shards are complete, and we've already received our LocalPrepared, we can set out
                        // LocalPrepared transaction as ready to propose ACCEPT. If we have not received
                        // the local LocalPrepared, the transition will happen when we receive the local
                        // block.
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL: Transaction is ready for propose AllPrepared({}, {}) Local Stage: {}",
                            tx_rec.transaction_id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );

                        tx_rec.add_pending_status_update(tx, local_leaf, TransactionPoolStage::LocalPrepared, true)?;
                        // TODO: there is a race condition between the local node receiving the foreign LocalPrepare and
                        // the leader proposing AllPrepare. If the latter comes first, this node
                        // will not vote on this block which leads inevitably to erroneous
                        // leader failures. Currently we simply vote ACCEPT on the block, with is ready == false, so we
                        // need to handle this here. When we confirm foreign proposals correctly, we can remove this.
                    } else if tx_rec.current_stage().is_all_prepared() &&
                        !tx_rec.is_ready() &&
                        tx_rec.evidence().all_input_addresses_justified()
                    {
                        // If the transaction is AllPrepared, we're waiting for all foreign proposals. Propose
                        // transaction once we have them.
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalPrepare) Transaction is ready for AllPrepared({}, {}) Local Stage: {}",
                            tx_rec.transaction_id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );
                        tx_rec.add_pending_status_update(tx, local_leaf, TransactionPoolStage::AllPrepared, true)?;
                    } else {
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL: Transaction is NOT ready for AllPrepared({}, {}) Local Stage: {}, All Justified: {}. Waiting for local proposal.",
                            tx_rec.transaction_id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage(),
                             tx_rec.evidence().all_input_addresses_justified()
                        );
                        tx_rec.add_pending_status_update(tx, local_leaf, tx_rec.current_stage(), false)?;
                    }
                },
                Command::LocalAccept(atom) => {
                    if !local_committee_info.includes_any_address(atom.evidence.substate_addresses_iter()) {
                        continue;
                    }
                    let Some(mut tx_rec) = self.transaction_pool.get(tx, local_leaf, &atom.id).optional()? else {
                        warn!(
                            target: LOG_TARGET,
                            "⚠️ NEVER HAPPEN: Foreign proposal received for transaction {} but this transaction is not in the pool.",
                            atom.id
                        );
                        continue;
                    };

                    if tx_rec.current_stage() > TransactionPoolStage::LocalAccepted {
                        warn!(
                            target: LOG_TARGET,
                            "⚠️ Foreign proposal received for transaction {} but current transaction stage is {}. Ignoring.",
                            tx_rec.transaction_id(), tx_rec.current_stage()
                        );
                        continue;
                    }

                    command_count += 1;

                    let remote_decision = atom.decision;
                    let local_decision = tx_rec.current_local_decision();
                    if remote_decision.is_abort() && local_decision.is_commit() {
                        info!(
                            target: LOG_TARGET,
                            "⚠️ Foreign ABORT {}. Update overall decision to ABORT. Local stage: {}, Leaf: {}",
                            tx_rec.transaction_id(), tx_rec.current_stage(), local_leaf
                        );
                    }

                    if !is_qc_saved {
                        // Save the QCs if it doesn't exist already, we'll reference the QC in subsequent blocks
                        block.justify().save(tx)?;
                        justify_qc.save(tx)?;
                        is_qc_saved = true;
                    }

                    // We need to add the justify QC to the evidence because the all prepare block could not include it
                    // yet
                    let mut foreign_evidence = atom.evidence.clone();
                    foreign_evidence.add_qc_evidence(foreign_committee_info, *justify_qc.id());

                    // Update the transaction record with any new information provided by this foreign block
                    tx_rec.update_remote_data(
                        tx,
                        remote_decision,
                        *justify_qc.id(),
                        foreign_committee_info,
                        foreign_evidence,
                    )?;

                    self.validate_and_add_pledges(
                        tx,
                        &tx_rec,
                        block.id(),
                        atom,
                        &mut block_pledge,
                        foreign_committee_info,
                    )?;

                    // Good debug info
                    // tx_rec.evidence().iter().for_each(|(addr, ev)| {
                    //     let includes_local = local_committee_info.includes_substate_address(addr);
                    //     log::error!(
                    //         target: LOG_TARGET,
                    //         "🐞 LOCALACCEPT EVIDENCE (l={}, f={}) {}: {}", includes_local, !includes_local, addr, ev
                    //     );
                    // });

                    if tx_rec.current_stage().is_new() {
                        // If the transaction is New, we're waiting for all foreign pledges. Propose transaction once we
                        // have them.
                        // CASE: Foreign SGs have pledged all inputs and executed the transaction, local SG is involved
                        // in the outputs
                        let transaction = tx_rec.get_transaction(&**tx)?;
                        let is_ready = local_committee_info.includes_substate_id(&transaction.to_receipt_id().into()) ||
                            transaction.has_any_local_inputs(local_committee_info) ||
                            transaction.has_all_foreign_input_pledges(&**tx, local_committee_info)?;
                        if is_ready {
                            info!(
                                target: LOG_TARGET,
                                "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalAccept) Transaction is ready for Prepare({}, {}) Local Stage: {}",
                                tx_rec.transaction_id(),
                                tx_rec.current_decision(),
                                tx_rec.current_stage()
                            );
                            tx_rec.add_pending_status_update(tx, local_leaf, TransactionPoolStage::New, true)?;
                        } else {
                            info!(
                                target: LOG_TARGET,
                                "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalAccept) Transaction is NOT ready for Prepare({}, {}) Local Stage: {}",
                                tx_rec.transaction_id(),
                                tx_rec.current_decision(),
                                tx_rec.current_stage()
                            );
                        }
                    } else if tx_rec.current_stage().is_local_accepted() && tx_rec.evidence().all_addresses_justified()
                    {
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL: Transaction is ready for propose ALL_ACCEPT({}, {}) Local Stage: {}",
                            tx_rec.transaction_id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );

                        tx_rec.add_pending_status_update(tx, local_leaf, TransactionPoolStage::LocalAccepted, true)?;
                    } else {
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL: Transaction is NOT ready for ALL_ACCEPT({}, {}) Local Stage: {}, All Justified: {}. Waiting for local proposal.",
                            tx_rec.transaction_id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage(),
                            tx_rec.evidence().all_addresses_justified()
                        );
                        tx_rec.add_pending_status_update(tx, local_leaf, tx_rec.current_stage(), false)?;
                    }
                },
                // Should never receive this
                Command::EndEpoch => {
                    warn!(
                        target: LOG_TARGET,
                        "❓️ NEVER HAPPEN: Foreign proposal received for block {} contains an EndEpoch command. This is invalid behaviour.",
                        block.id()
                    );
                    continue;
                },
                // TODO(perf): Can we find a way to exclude these unused commands to reduce message size?
                Command::AllAccept(_) |
                Command::SomeAccept(_) |
                Command::AllPrepare(_) |
                Command::SomePrepare(_) |
                Command::ForeignProposal(_) |
                Command::Prepare(_) |
                Command::LocalOnly(_) => {
                    // Disregard
                    continue;
                },
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

    fn validate_and_add_pledges(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        tx_rec: &TransactionPoolRecord,
        block_id: &BlockId,
        atom: &TransactionAtom,
        block_pledge: &mut BlockPledge,
        foreign_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        #[allow(clippy::mutable_key_type)]
        let maybe_pledges = if atom.decision.is_commit() {
            let pledges = block_pledge.remove_transaction_pledges(&atom.id).ok_or_else(|| {
                HotStuffError::ForeignNodeOmittedTransactionPledges {
                    foreign_block_id: *block_id,
                    transaction_id: atom.id,
                }
            })?;

            // Validate that provided evidence is correct
            // TODO: there are a lot of validations to be done on evidence and the foreign block in general,
            // this is here as a sanity check and should change to not be a fatal error in consensus
            for pledge in &pledges {
                let address = pledge.versioned_substate_id().to_substate_address();
                let evidence =
                    atom.evidence
                        .get(&address)
                        .ok_or_else(|| ProposalValidationError::ForeignInvalidPledge {
                            block_id: *block_id,
                            transaction_id: atom.id,
                            details: format!("Pledge {pledge} for address {address} not found in evidence"),
                        })?;
                if evidence.lock.is_output() && pledge.is_input() {
                    return Err(ProposalValidationError::ForeignInvalidPledge {
                        block_id: *block_id,
                        transaction_id: atom.id,
                        details: format!("Pledge {pledge} is an input but evidence is an output for address {address}"),
                    }
                    .into());
                }
                if !evidence.lock.is_output() && pledge.is_output() {
                    return Err(ProposalValidationError::ForeignInvalidPledge {
                        block_id: *block_id,
                        transaction_id: atom.id,
                        details: format!("Pledge {pledge} is an output but evidence is an input for address {address}"),
                    }
                    .into());
                }
            }
            Some(pledges)
        } else {
            if block_pledge.remove_transaction_pledges(&atom.id).is_some() {
                return Err(ProposalValidationError::ForeignInvalidPledge {
                    block_id: *block_id,
                    transaction_id: atom.id,
                    details: "Remote decided ABORT but provided pledges".to_string(),
                }
                .into());
            }
            None
        };

        if let Some(pledges) = maybe_pledges {
            // If the foreign shard has committed the transaction, we can add the pledges to the transaction
            // record
            tx_rec.add_foreign_pledges(tx, foreign_committee_info.shard_group(), pledges)?;
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
