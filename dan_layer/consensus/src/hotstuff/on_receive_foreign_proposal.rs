//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::{committee::CommitteeInfo, ShardGroup};
use tari_dan_storage::{
    consensus_models::{Block, ForeignProposal, ForeignReceiveCounters},
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{error::HotStuffError, pacemaker_handle::PaceMakerHandle, ProposalValidationError},
    messages::ForeignProposalMessage,
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_foreign_proposal";

#[derive(Clone)]
pub struct OnReceiveForeignProposalHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    pacemaker: PaceMakerHandle,
}

impl<TConsensusSpec> OnReceiveForeignProposalHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        pacemaker: PaceMakerHandle,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            pacemaker,
        }
    }

    pub async fn handle(
        &mut self,
        message: ForeignProposalMessage,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let foreign_committee_info = self
            .epoch_manager
            .get_committee_info_by_validator_public_key(message.block.epoch(), message.block.proposed_by())
            .await?;
        self.validate_and_save(message, local_committee_info, &foreign_committee_info)?;
        Ok(())
    }

    pub fn validate_and_save(
        &mut self,
        message: ForeignProposalMessage,
        local_committee_info: &CommitteeInfo,
        foreign_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let proposal = ForeignProposal::from(message);

        if self.store.with_read_tx(|tx| proposal.exists(tx))? {
            // This is expected behaviour, we may receive the same foreign proposal multiple times
            debug!(
                target: LOG_TARGET,
                "FOREIGN PROPOSAL: Already received proposal for block {}",
                proposal.block().id(),
            );
            return Ok(());
        }

        // TODO: validate justify_qc
        let mut foreign_receive_counter = self
            .store
            .with_read_tx(|tx| ForeignReceiveCounters::get_or_default(tx))?;

        if let Err(err) = self.validate_proposed_block(
            proposal.block(),
            foreign_committee_info.shard_group(),
            local_committee_info.shard_group(),
            &foreign_receive_counter,
        ) {
            warn!(
                target: LOG_TARGET,
                "⚠️ FOREIGN PROPOSAL: Invalid proposal: {}. Ignoring.",
                err
            );
            // Invalid blocks should not cause the state machine to transition to Error
            return Ok(());
        }

        foreign_receive_counter.increment_group(foreign_committee_info.shard_group());

        // Justify QC must justify the block
        if proposal.justify_qc().block_id() != proposal.block().id() {
            warn!(
                target: LOG_TARGET,
                "⚠️ FOREIGN PROPOSAL: Justify QC block id does not match the block id. Justify QC block id: {}, block id: {}",
                proposal.justify_qc().block_id(),
                proposal.block().id(),
            );
            return Ok(());
        }

        info!(
            target: LOG_TARGET,
            "🧩 Receive FOREIGN PROPOSAL for block {}, justify_qc: {}",
            proposal.block(),
            proposal.justify_qc(),
        );

        self.store.with_write_tx(|tx| {
            foreign_receive_counter.save(tx)?;
            proposal.upsert(tx, None)
        })?;

        // Foreign proposals to propose
        self.pacemaker.on_beat();

        Ok(())
    }

    // #[allow(clippy::too_many_lines)]
    // fn process_foreign_block(
    //     &self,
    //     tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
    //     foreign_proposal: ForeignProposal,
    //     foreign_committee_info: &CommitteeInfo,
    //     local_committee_info: &CommitteeInfo,
    // ) -> Result<(), HotStuffError> {
    //     let ForeignProposal {
    //         block,
    //         justify_qc,
    //         mut block_pledge,
    //         ..
    //     } = foreign_proposal;
    //     let local_leaf = LeafBlock::get(&**tx)?;
    //     // We only want to save the QC once if applicable
    //     let mut command_count = 0usize;
    //
    //     for cmd in block.commands() {
    //         match cmd {
    //             Command::LocalPrepare(atom) => {
    //                 if !local_committee_info.includes_any_address(atom.evidence.substate_addresses_iter()) {
    //                     continue;
    //                 }
    //
    //                 debug!(
    //                     target: LOG_TARGET,
    //                     "🧩 FOREIGN PROPOSAL: Command: LocalPrepare({}, {}), block: {}",
    //                     atom.id,atom.decision, block.id(),
    //                 );
    //
    //                 let Some(mut tx_rec) = self.transaction_pool.get(tx, local_leaf, &atom.id).optional()? else {
    //                     // If this happens, it could be a bug in the foreign missing transaction handling
    //                     warn!(
    //                         target: LOG_TARGET,
    //                         "⚠️ NEVER HAPPEN: Foreign proposal received for transaction {} but this transaction is
    // not in the pool.",                         atom.id
    //                     );
    //                     continue;
    //                 };
    //
    //                 if tx_rec.current_stage() > TransactionPoolStage::LocalPrepared {
    //                     // TODO: This can happen if the foreign shard group is only responsible for outputs (the
    // input                     // SGs have already progressed to LocalAccept) in which case it is safe to ignore
    // this command.                     // However we should not send the proposal in the first place (assuming it
    // does not involve any                     // other shard-applicable transactions).
    //                     warn!(
    //                         target: LOG_TARGET,
    //                         "⚠️ Foreign LocalPrepare proposal ({}) received LOCAL_PREPARE for transaction {} but
    // current transaction stage is {}. Ignoring.",                         block,
    //                         tx_rec.transaction_id(), tx_rec.current_stage()
    //                     );
    //                     continue;
    //                 }
    //
    //                 command_count += 1;
    //
    //                 let remote_decision = atom.decision;
    //                 let local_decision = tx_rec.current_decision();
    //                 if remote_decision.is_abort() && local_decision.is_commit() {
    //                     info!(
    //                         target: LOG_TARGET,
    //                         "⚠️ Foreign committee ABORT transaction {}. Update overall decision to ABORT. Local
    // stage: {}, Leaf: {}",                         tx_rec.transaction_id(), tx_rec.current_stage(), local_leaf
    //                     );
    //                 }
    //
    //                 // We need to add the justify QC to the evidence because the all prepare block could not include
    // it                 // yet
    //                 let mut foreign_evidence = atom.evidence.clone();
    //                 foreign_evidence.add_qc_evidence(foreign_committee_info, *justify_qc.id());
    //
    //                 // Update the transaction record with any new information provided by this foreign block
    //                 tx_rec.update_remote_data(
    //                     tx,
    //                     remote_decision,
    //                     *justify_qc.id(),
    //                     foreign_committee_info,
    //                     foreign_evidence,
    //                 )?;
    //
    //                 self.validate_and_add_pledges(
    //                     tx,
    //                     &tx_rec,
    //                     block.id(),
    //                     atom,
    //                     &mut block_pledge,
    //                     foreign_committee_info,
    //                 )?;
    //
    //                 if tx_rec.current_stage().is_new() {
    //                     info!(
    //                         target: LOG_TARGET,
    //                         "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalPrepare) Transaction is ready for
    // Prepare({}, {}) Local Stage: {}",                         tx_rec.transaction_id(),
    //                         tx_rec.current_decision(),
    //                         tx_rec.current_stage()
    //                     );
    //                     // If the transaction is New, we're waiting for all foreign pledges. Propose transaction once
    // we                     // have them.
    //
    //                     // CASE: One foreign SG is involved in all inputs and executed the transaction, local SG is
    //                     // involved in the outputs
    //                     let transaction = tx_rec.get_transaction(&**tx)?;
    //                     let is_ready = local_committee_info.includes_substate_id(&transaction.to_receipt_id().into())
    // ||                         transaction.has_any_local_inputs(local_committee_info) ||
    //                         transaction.has_all_foreign_input_pledges(&**tx, local_committee_info)?;
    //
    //                     if is_ready {
    //                         info!(
    //                             target: LOG_TARGET,
    //                             "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalPrepare) Transaction is ready for
    // Prepare({}, {}) Local Stage: {}",                             tx_rec.transaction_id(),
    //                             tx_rec.current_decision(),
    //                             tx_rec.current_stage()
    //                         );
    //                         tx_rec.add_pending_status_update(tx, local_leaf, TransactionPoolStage::New, true)?;
    //                     } else {
    //                         info!(
    //                             target: LOG_TARGET,
    //                             "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalPrepare) Transaction is NOT ready
    // for Prepare({}, {}) Local Stage: {}",                             tx_rec.transaction_id(),
    //                             tx_rec.current_decision(),
    //                             tx_rec.current_stage()
    //                         );
    //                     }
    //                 } else if tx_rec.current_stage().is_local_prepared() &&
    //                     tx_rec.evidence().all_input_addresses_justified()
    //                 {
    //                     // If all shards are complete, and we've already received our LocalPrepared, we can set out
    //                     // LocalPrepared transaction as ready to propose ACCEPT. If we have not received
    //                     // the local LocalPrepared, the transition will happen when we receive the local
    //                     // block.
    //                     info!(
    //                         target: LOG_TARGET,
    //                         "🧩 FOREIGN PROPOSAL: Transaction is ready for propose AllPrepared({}, {}) Local Stage:
    // {}",                         tx_rec.transaction_id(),
    //                         tx_rec.current_decision(),
    //                         tx_rec.current_stage()
    //                     );
    //
    //                     tx_rec.add_pending_status_update(tx, local_leaf, TransactionPoolStage::LocalPrepared, true)?;
    //                     // TODO: there is a race condition between the local node receiving the foreign LocalPrepare
    // and                     // the leader proposing AllPrepare. If the latter comes first, this node
    //                     // will not vote on this block which leads inevitably to erroneous
    //                     // leader failures. Currently we simply vote ACCEPT on the block, with is ready == false, so
    // we                     // need to handle this here. When we confirm foreign proposals correctly, we can
    // remove this.                 } else {
    //                     info!(
    //                         target: LOG_TARGET,
    //                         "🧩 FOREIGN PROPOSAL: Transaction is NOT ready for AllPrepared({}, {}) Local Stage: {},
    // All Justified: {}. Waiting for local proposal.",                         tx_rec.transaction_id(),
    //                         tx_rec.current_decision(),
    //                         tx_rec.current_stage(),
    //                          tx_rec.evidence().all_input_addresses_justified()
    //                     );
    //                     tx_rec.add_pending_status_update(tx, local_leaf, tx_rec.current_stage(), tx_rec.is_ready())?;
    //                 }
    //             },
    //             Command::LocalAccept(atom) => {
    //                 if !local_committee_info.includes_any_address(atom.evidence.substate_addresses_iter()) {
    //                     continue;
    //                 }
    //
    //                 debug!(
    //                     target: LOG_TARGET,
    //                     "🧩 FOREIGN PROPOSAL: Command: LocalAccept({}, {}), block: {}",
    //                     atom.id, atom.decision, block.id(),
    //                 );
    //
    //                 let Some(mut tx_rec) = self.transaction_pool.get(tx, local_leaf, &atom.id).optional()? else {
    //                     warn!(
    //                         target: LOG_TARGET,
    //                         "⚠️ NEVER HAPPEN: Foreign proposal received for transaction {} but this transaction is
    // not in the pool.",                         atom.id
    //                     );
    //                     continue;
    //                 };
    //
    //                 if tx_rec.current_stage() > TransactionPoolStage::LocalAccepted {
    //                     warn!(
    //                         target: LOG_TARGET,
    //                         "⚠️ Foreign proposal {} received LOCAL_ACCEPT for transaction {} but current transaction
    // stage is {}. Ignoring.",                         block,
    //                         tx_rec.transaction_id(),
    //                         tx_rec.current_stage(),
    //                     );
    //                     continue;
    //                 }
    //
    //                 command_count += 1;
    //
    //                 let remote_decision = atom.decision;
    //                 let local_decision = tx_rec.current_local_decision();
    //                 if remote_decision.is_abort() && local_decision.is_commit() {
    //                     info!(
    //                         target: LOG_TARGET,
    //                         "⚠️ Foreign ABORT {}. Update overall decision to ABORT. Local stage: {}, Leaf: {}",
    //                         tx_rec.transaction_id(), tx_rec.current_stage(), local_leaf
    //                     );
    //                 }
    //
    //                 // We need to add the justify QC to the evidence because the all prepare block could not include
    // it                 // yet
    //                 let mut foreign_evidence = atom.evidence.clone();
    //                 foreign_evidence.add_qc_evidence(foreign_committee_info, *justify_qc.id());
    //
    //                 // Update the transaction record with any new information provided by this foreign block
    //                 tx_rec.update_remote_data(
    //                     tx,
    //                     remote_decision,
    //                     *justify_qc.id(),
    //                     foreign_committee_info,
    //                     foreign_evidence,
    //                 )?;
    //
    //                 self.validate_and_add_pledges(
    //                     tx,
    //                     &tx_rec,
    //                     block.id(),
    //                     atom,
    //                     &mut block_pledge,
    //                     foreign_committee_info,
    //                 )?;
    //
    //                 // Good debug info
    //                 // tx_rec.evidence().iter().for_each(|(addr, ev)| {
    //                 //     let includes_local = local_committee_info.includes_substate_address(addr);
    //                 //     log::error!(
    //                 //         target: LOG_TARGET,
    //                 //         "🐞 LOCALACCEPT EVIDENCE (l={}, f={}) {}: {}", includes_local, !includes_local, addr,
    // ev                 //     );
    //                 // });
    //
    //                 if tx_rec.current_stage().is_new() {
    //                     // If the transaction is New, we're waiting for all foreign pledges. Propose transaction once
    // we                     // have them.
    //                     // CASE: Foreign SGs have pledged all inputs and executed the transaction, local SG is
    // involved                     // in the outputs
    //                     let transaction = tx_rec.get_transaction(&**tx)?;
    //                     let is_ready = local_committee_info.includes_substate_id(&transaction.to_receipt_id().into())
    // ||                         transaction.has_any_local_inputs(local_committee_info) ||
    //                         transaction.has_all_foreign_input_pledges(&**tx, local_committee_info)?;
    //                     if is_ready {
    //                         info!(
    //                             target: LOG_TARGET,
    //                             "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalAccept) Transaction is ready for
    // Prepare({}, {}) Local Stage: {}",                             tx_rec.transaction_id(),
    //                             tx_rec.current_decision(),
    //                             tx_rec.current_stage()
    //                         );
    //                         tx_rec.add_pending_status_update(tx, local_leaf, TransactionPoolStage::New, true)?;
    //                     } else {
    //                         info!(
    //                             target: LOG_TARGET,
    //                             "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalAccept) Transaction is NOT ready
    // for Prepare({}, {}) Local Stage: {}",                             tx_rec.transaction_id(),
    //                             tx_rec.current_decision(),
    //                             tx_rec.current_stage()
    //                         );
    //                     }
    //                 } else if tx_rec.current_stage().is_local_accepted() &&
    // tx_rec.evidence().all_addresses_justified()                 {
    //                     info!(
    //                         target: LOG_TARGET,
    //                         "🧩 FOREIGN PROPOSAL: Transaction is ready for propose ALL_ACCEPT({}, {}) Local Stage:
    // {}",                         tx_rec.transaction_id(),
    //                         tx_rec.current_decision(),
    //                         tx_rec.current_stage()
    //                     );
    //
    //                     tx_rec.add_pending_status_update(tx, local_leaf, TransactionPoolStage::LocalAccepted, true)?;
    //                 } else {
    //                     info!(
    //                         target: LOG_TARGET,
    //                         "🧩 FOREIGN PROPOSAL: Transaction is NOT ready for ALL_ACCEPT({}, {}) Local Stage: {},
    // All Justified: {}. Waiting for local proposal.",                         tx_rec.transaction_id(),
    //                         tx_rec.current_decision(),
    //                         tx_rec.current_stage(),
    //                         tx_rec.evidence().all_addresses_justified()
    //                     );
    //                     // Still need to update the evidence
    //                     tx_rec.add_pending_status_update(tx, local_leaf, tx_rec.current_stage(), tx_rec.is_ready())?;
    //                 }
    //             },
    //             // Should never receive this
    //             Command::EndEpoch => {
    //                 warn!(
    //                     target: LOG_TARGET,
    //                     "❓️ NEVER HAPPEN: Foreign proposal received for block {} contains an EndEpoch command. This
    // is invalid behaviour.",                     block.id()
    //                 );
    //                 continue;
    //             },
    //             // TODO(perf): Can we find a way to exclude these unused commands to reduce message size?
    //             Command::AllAccept(_) |
    //             Command::SomeAccept(_) |
    //             Command::AllPrepare(_) |
    //             Command::SomePrepare(_) |
    //             Command::Prepare(_) |
    //             Command::LocalOnly(_) |
    //             Command::ForeignProposal(_) |
    //             Command::MintConfidentialOutput(_) => {
    //                 // Disregard
    //                 continue;
    //             },
    //         }
    //     }
    //
    //     info!(
    //         target: LOG_TARGET,
    //         "🧩 FOREIGN PROPOSAL: Processed {} commands from foreign block {}",
    //         command_count,
    //         block.id()
    //     );
    //     if command_count == 0 {
    //         warn!(
    //             target: LOG_TARGET,
    //             "⚠️ FOREIGN PROPOSAL: No commands were applicable for foreign block {}. Ignoring.",
    //             block.id()
    //         );
    //     }
    //
    //     Ok(())
    // }
    //
    // fn validate_and_add_pledges(
    //     &self,
    //     tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
    //     tx_rec: &TransactionPoolRecord,
    //     block_id: &BlockId,
    //     atom: &TransactionAtom,
    //     block_pledge: &mut BlockPledge,
    //     foreign_committee_info: &CommitteeInfo,
    // ) -> Result<(), HotStuffError> {
    //     #[allow(clippy::mutable_key_type)]
    //     let maybe_pledges = if atom.decision.is_commit() {
    //         let pledges = block_pledge.remove_transaction_pledges(&atom.id).ok_or_else(|| {
    //             HotStuffError::ForeignNodeOmittedTransactionPledges {
    //                 foreign_block_id: *block_id,
    //                 transaction_id: atom.id,
    //             }
    //         })?;
    //
    //         // Validate that provided evidence is correct
    //         // TODO: there are a lot of validations to be done on evidence and the foreign block in general,
    //         // this is here as a sanity check and should change to not be a fatal error in consensus
    //         for pledge in &pledges {
    //             let address = pledge.versioned_substate_id().to_substate_address();
    //             let evidence =
    //                 atom.evidence
    //                     .get(&address)
    //                     .ok_or_else(|| ProposalValidationError::ForeignInvalidPledge {
    //                         block_id: *block_id,
    //                         transaction_id: atom.id,
    //                         details: format!("Pledge {pledge} for address {address} not found in evidence"),
    //                     })?;
    //             if evidence.lock.is_output() && pledge.is_input() {
    //                 return Err(ProposalValidationError::ForeignInvalidPledge {
    //                     block_id: *block_id,
    //                     transaction_id: atom.id,
    //                     details: format!("Pledge {pledge} is an input but evidence is an output for address
    // {address}"),                 }
    //                 .into());
    //             }
    //             if !evidence.lock.is_output() && pledge.is_output() {
    //                 return Err(ProposalValidationError::ForeignInvalidPledge {
    //                     block_id: *block_id,
    //                     transaction_id: atom.id,
    //                     details: format!("Pledge {pledge} is an output but evidence is an input for address
    // {address}"),                 }
    //                 .into());
    //             }
    //         }
    //         Some(pledges)
    //     } else {
    //         if block_pledge.remove_transaction_pledges(&atom.id).is_some() {
    //             return Err(ProposalValidationError::ForeignInvalidPledge {
    //                 block_id: *block_id,
    //                 transaction_id: atom.id,
    //                 details: "Remote decided ABORT but provided pledges".to_string(),
    //             }
    //             .into());
    //         }
    //         None
    //     };
    //
    //     if let Some(pledges) = maybe_pledges {
    //         // If the foreign shard has committed the transaction, we can add the pledges to the transaction
    //         // record
    //         tx_rec.add_foreign_pledges(tx, foreign_committee_info.shard_group(), pledges)?;
    //     }
    //
    //     Ok(())
    // }

    fn validate_proposed_block(
        &self,
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
                proposed_by: candidate_block.proposed_by().to_string(),
                hash: *candidate_block.id(),
            });
        }

        let calculated_hash = candidate_block.calculate_hash().into();
        if calculated_hash != *candidate_block.id() {
            return Err(ProposalValidationError::NodeHashMismatch {
                proposed_by: candidate_block.proposed_by().to_string(),
                hash: *candidate_block.id(),
                calculated_hash,
            });
        }

        // TODO: validate justify signatures
        // self.validate_qc(candidate_block.justify(), committee)?;

        Ok(())
    }
}
