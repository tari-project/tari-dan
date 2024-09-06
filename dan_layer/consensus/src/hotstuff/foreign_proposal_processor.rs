//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::{committee::CommitteeInfo, ToSubstateAddress};
use tari_dan_storage::{
    consensus_models::{
        BlockId,
        BlockPledge,
        Command,
        ForeignProposal,
        LeafBlock,
        LockedBlock,
        TransactionAtom,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionRecord,
    },
    StateStoreReadTransaction,
};
use tari_engine_types::commit_result::RejectReason;
use tari_transaction::TransactionId;

use crate::hotstuff::{block_change_set::ProposedBlockChangeSet, error::HotStuffError, ProposalValidationError};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::foreign_proposal_processor";

#[allow(clippy::too_many_lines)]
pub fn process_foreign_block<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    local_leaf: &LeafBlock,
    locked_block: &LockedBlock,
    proposal: ForeignProposal,
    foreign_committee_info: &CommitteeInfo,
    local_committee_info: &CommitteeInfo,
    proposed_block_change_set: &mut ProposedBlockChangeSet,
) -> Result<(), HotStuffError> {
    assert_eq!(
        proposal.block.shard_group(),
        foreign_committee_info.shard_group(),
        "Foreign proposal shard group does not match the foreign committee shard group"
    );
    info!(
        target: LOG_TARGET,
        "🧩 Processing FOREIGN PROPOSAL for block {}, justify_qc: {}",
        proposal.block(),
        proposal.justify_qc(),
    );

    let ForeignProposal {
        block,
        justify_qc,
        mut block_pledge,
        ..
    } = proposal;
    let mut command_count = 0usize;

    for cmd in block.commands() {
        match cmd {
            Command::LocalPrepare(atom) => {
                if !local_committee_info.includes_any_address(atom.evidence.substate_addresses_iter()) {
                    continue;
                }

                debug!(
                    target: LOG_TARGET,
                    "🧩 FOREIGN PROPOSAL: Command: LocalPrepare({}, {}), block: {}",
                    atom.id,atom.decision, block.id(),
                );

                let Some(mut tx_rec) =
                    proposed_block_change_set.get_transaction(tx, locked_block, local_leaf, &atom.id)?
                else {
                    // If this happens, it could be a bug in the foreign missing transaction handling
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ NEVER HAPPEN: Foreign proposal received for transaction {} but this transaction is not in the pool.",
                        atom.id
                    );
                    continue;
                };

                let is_shard_group_output_only = tx_rec.evidence().is_committee_output_only(foreign_committee_info);
                if is_shard_group_output_only {
                    // If the shard group is only involved in the outputs, we can ignore Prepare commands.
                    debug!(
                        target: LOG_TARGET,
                        "❓️ Foreign LocalPrepare proposal ({}) received LOCAL_PREPARE for transaction {} stage: {} but the foreign shard group is only involved in outputs. Ignoring.",
                        block,
                        tx_rec.transaction_id(),
                        tx_rec.current_stage()
                    );

                    continue;
                }

                if tx_rec.current_stage() > TransactionPoolStage::LocalPrepared {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Foreign LocalPrepare proposal ({}) received LOCAL_PREPARE for transaction {} but current transaction stage is {}. Ignoring.",
                        block,
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
                        "⚠️ Foreign committee ABORT transaction {}. Update overall decision to ABORT. Local stage: {}, Leaf: {}",
                        tx_rec.transaction_id(), tx_rec.current_stage(), local_leaf
                    );

                    // Add an abort execution since we previously decided to commit
                    let mut transaction = TransactionRecord::get(tx, tx_rec.transaction_id())?;
                    transaction.set_abort_reason(RejectReason::ForeignShardGroupDecidedToAbort(format!(
                        "Foreign shard group {} decided to abort the transaction",
                        foreign_committee_info.shard_group()
                    )));
                    let exec = transaction.into_execution().expect("ABORT set above");
                    proposed_block_change_set.add_transaction_execution(exec)?;
                }

                // We need to add the justify QC to the evidence because the all prepare block could not include it
                // yet
                let foreign_evidence = atom.evidence.clone();

                // Update the transaction record with any new information provided by this foreign block
                tx_rec
                    .evidence_mut()
                    .update(foreign_evidence.iter().map(|(addr, e)| (*addr, e.lock)))
                    .add_prepare_qc_evidence(foreign_committee_info, *justify_qc.id());
                tx_rec.set_remote_decision(remote_decision);

                validate_and_add_pledges(
                    tx_rec.transaction_id(),
                    block.id(),
                    atom,
                    &mut block_pledge,
                    foreign_committee_info,
                    proposed_block_change_set,
                )?;

                // tx_rec.evidence().iter().for_each(|(addr, ev)| {
                //     let includes_local = local_committee_info.includes_substate_address(addr);
                //     log::error!(
                //         target: LOG_TARGET,
                //         "🐞 LOCALPREPARE EVIDENCE (l={}, f={}) {}: {}", includes_local, !includes_local, addr, ev
                //     );
                // });

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
                    let is_ready = local_committee_info.includes_substate_id(&tx_rec.to_receipt_id().into()) ||
                        tx_rec.has_any_local_inputs(local_committee_info) ||
                        has_all_foreign_input_pledges(tx, &tx_rec, local_committee_info, proposed_block_change_set)?;

                    if is_ready {
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalPrepare) Transaction is ready for Prepare({}, {}) Local Stage: {}",
                            tx_rec.transaction_id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );
                        tx_rec.set_next_stage(TransactionPoolStage::New, true)?;
                        proposed_block_change_set.set_next_transaction_update(tx_rec)?;
                    } else {
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalPrepare) Transaction is NOT ready for Prepare({}, {}) Local Stage: {}",
                            tx_rec.transaction_id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );
                    }
                } else if tx_rec.current_stage().is_local_prepared() && tx_rec.evidence().all_input_addresses_prepared()
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

                    tx_rec.set_next_stage(TransactionPoolStage::LocalPrepared, true)?;
                    proposed_block_change_set.set_next_transaction_update(tx_rec)?;
                } else {
                    info!(
                        target: LOG_TARGET,
                        "🧩 FOREIGN PROPOSAL: Transaction is NOT ready for AllPrepared({}, {}) Local Stage: {}, All Justified: {}. Waiting for local proposal.",
                        tx_rec.transaction_id(),
                        tx_rec.current_decision(),
                        tx_rec.current_stage(),
                         tx_rec.evidence().all_input_addresses_prepared()
                    );
                    // Update the evidence
                    proposed_block_change_set.set_next_transaction_update(tx_rec)?;
                }
            },
            Command::LocalAccept(atom) => {
                if !local_committee_info.includes_any_address(atom.evidence.substate_addresses_iter()) {
                    continue;
                }

                debug!(
                    target: LOG_TARGET,
                    "🧩 FOREIGN PROPOSAL: Command: LocalAccept({}, {}), block: {}",
                    atom.id, atom.decision, block.id(),
                );

                let Some(mut tx_rec) =
                    proposed_block_change_set.get_transaction(tx, locked_block, local_leaf, &atom.id)?
                else {
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
                        "⚠️ Foreign proposal {} received LOCAL_ACCEPT for transaction {} but current transaction stage is {}. Ignoring.",
                        block,
                        tx_rec.transaction_id(),
                        tx_rec.current_stage(),
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

                // We need to add the justify QC to the evidence because the all prepare block could not include it
                // yet
                let foreign_evidence = atom.evidence.clone();

                // Update the transaction record with any new information provided by this foreign block
                tx_rec
                    .evidence_mut()
                    .update(foreign_evidence.iter().map(|(addr, e)| (*addr, e.lock)))
                    .add_accept_qc_evidence(foreign_committee_info, *justify_qc.id());
                tx_rec.set_remote_decision(remote_decision);

                validate_and_add_pledges(
                    tx_rec.transaction_id(),
                    block.id(),
                    atom,
                    &mut block_pledge,
                    foreign_committee_info,
                    proposed_block_change_set,
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
                    let is_ready = local_committee_info.includes_substate_id(&tx_rec.to_receipt_id().into()) ||
                        tx_rec.has_any_local_inputs(local_committee_info) ||
                        has_all_foreign_input_pledges(tx, &tx_rec, local_committee_info, proposed_block_change_set)?;
                    if is_ready {
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalAccept) Transaction is ready for Prepare({}, {}) Local Stage: {}",
                            tx_rec.transaction_id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );

                        tx_rec.set_next_stage(TransactionPoolStage::New, true)?;
                        proposed_block_change_set.set_next_transaction_update(tx_rec)?;
                    } else {
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL: (Initial sequence from LocalAccept) Transaction is NOT ready for Prepare({}, {}) Local Stage: {}",
                            tx_rec.transaction_id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );

                        proposed_block_change_set.set_next_transaction_update(tx_rec)?;
                    }
                } else if tx_rec.current_stage().is_local_accepted() && tx_rec.evidence().all_addresses_justified() {
                    info!(
                        target: LOG_TARGET,
                        "🧩 FOREIGN PROPOSAL: Transaction is ready for propose ALL_ACCEPT({}, {}) Local Stage: {}",
                        tx_rec.transaction_id(),
                        tx_rec.current_decision(),
                        tx_rec.current_stage()
                    );

                    tx_rec.set_next_stage(TransactionPoolStage::LocalAccepted, true)?;
                    proposed_block_change_set.set_next_transaction_update(tx_rec)?;
                } else {
                    info!(
                        target: LOG_TARGET,
                        "🧩 FOREIGN PROPOSAL: Transaction is NOT ready for ALL_ACCEPT({}, {}) Local Stage: {}, All Justified: {}. Waiting for local proposal.",
                        tx_rec.transaction_id(),
                        tx_rec.current_decision(),
                        tx_rec.current_stage(),
                        tx_rec.evidence().all_addresses_justified()
                    );
                    // Still need to update the evidence
                    proposed_block_change_set.set_next_transaction_update(tx_rec)?;
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
            Command::Prepare(_) |
            Command::LocalOnly(_) |
            Command::ForeignProposal(_) |
            Command::MintConfidentialOutput(_) => {
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
    transaction_id: &TransactionId,
    block_id: &BlockId,
    atom: &TransactionAtom,
    block_pledge: &mut BlockPledge,
    foreign_committee_info: &CommitteeInfo,
    proposed_block_change_set: &mut ProposedBlockChangeSet,
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
        if block_pledge.contains(&atom.id) {
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
        proposed_block_change_set.add_foreign_pledges(transaction_id, foreign_committee_info.shard_group(), pledges);
    }

    Ok(())
}

fn has_all_foreign_input_pledges<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    tx_rec: &TransactionPoolRecord,
    local_committee_info: &CommitteeInfo,
    proposed_block_change_set: &ProposedBlockChangeSet,
) -> Result<bool, HotStuffError> {
    let foreign_inputs = tx_rec
        .evidence()
        .iter()
        .filter(|(addr, ev)| !ev.lock.is_output() && !local_committee_info.includes_substate_address(addr))
        .map(|(addr, _)| addr);

    let current_pledges = proposed_block_change_set.get_foreign_pledges(tx_rec.transaction_id());

    for addr in foreign_inputs {
        // Check the current block change set to see if the pledge is included
        if current_pledges.clone().any(|pledge| pledge.satisfies_address(addr)) {
            continue;
        }

        if tx.foreign_substate_pledges_exists_for_address(tx_rec.transaction_id(), addr)? {
            continue;
        }
        debug!(
            target: LOG_TARGET,
            "Transaction {} is missing a pledge for input {}",
            tx_rec.transaction_id(),
            addr
        );
        return Ok(false);
    }

    Ok(true)
}
