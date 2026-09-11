//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_consensus_types::{BlockId, Decision, LeafBlock, PcId, ProposalCertificate, ValidatorSignatureBytes};
use tari_crypto::tari_utilities::ByteArray;
use tari_engine_types::commit_result::RejectReason;
use tari_ootle_common_types::{
    NumPreshards,
    ShardGroup,
    SubstateAddress,
    committee::CommitteeInfo,
    displayable::Displayable,
    optional::Optional,
};
use tari_ootle_storage::{
    StateStoreReadTransaction,
    consensus_models::{
        BlockPledge,
        Command,
        Evidence,
        ForeignProposal,
        LockedSubstateValue,
        TransactionAtom,
        TransactionExecution,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionRecord,
    },
};
use tari_ootle_transaction::TransactionId;
use tari_template_lib_types::crypto::{RistrettoPublicKeyBytes, Scalar32Bytes, SchnorrSignatureBytes};

use crate::{
    hotstuff::{
        ProposalValidationError,
        block_change_set::ProposedBlockChangeSet,
        error::HotStuffError,
        substate_store::PendingSubstateStore,
    },
    tracing::TraceTimer,
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::foreign_proposal_processor";

#[allow(clippy::too_many_lines)]
pub fn process_foreign_block<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    local_leaf: &LeafBlock,
    proposal: &ForeignProposal,
    local_committee_info: &CommitteeInfo,
    substate_store: &mut PendingSubstateStore<TTx>,
    proposed_block_change_set: &mut ProposedBlockChangeSet,
) -> Result<(), HotStuffError> {
    let _timer = TraceTimer::info(LOG_TARGET, "process_foreign_block");

    let foreign_shard_group = proposal.shard_group_unchecked();
    info!(
        target: LOG_TARGET,
        "🧩 Processing FOREIGN PROPOSAL {}",
        proposal,
    );
    // Used for logging purposes
    let foreign_block_id = proposal.calculate_block_id();
    let foreign_proposal_as_leaf = LeafBlock {
        block_id: foreign_block_id,
        height: proposal.height(),
        epoch: proposal.epoch(),
        shard_group: foreign_shard_group,
    };

    let block_pledge = proposal.block_pledge();
    let mut command_count = 0usize;

    for cmd in proposal.commit_proof().applicable_commands_iter() {
        match cmd {
            Command::LocalPrepare(atom) => {
                if !atom.evidence.has(&local_committee_info.shard_group()) ||
                    // Foreign output-only nodes should not do a local prepare, if they do we ignore it
                    atom.evidence
                        .is_committee_output_only(foreign_shard_group)
                {
                    warn!(
                        target: LOG_TARGET,
                        "🧩 FOREIGN PROPOSAL {foreign_shard_group}: Command: LocalPrepare({}, {}), block: {} not relevant to local committee",
                        atom.id, atom.decision, foreign_block_id,
                    );
                    continue;
                }

                debug!(
                    target: LOG_TARGET,
                    "🧩 FOREIGN PROPOSAL {foreign_shard_group}: Command: LocalPrepare({}, {}), block: {}",
                    atom.id,atom.decision, foreign_block_id,
                );

                let Some(mut tx_rec) = get_or_sequence_transaction(
                    tx,
                    local_committee_info.num_preshards(),
                    local_committee_info.num_committees(),
                    &atom.id,
                    local_leaf,
                    &foreign_block_id,
                    proposed_block_change_set,
                )?
                else {
                    // TODO: evaluate this case - None is returned if the transaction is already finalized, which could
                    // be a race condition however, gracefully handling an error is more correct
                    // than just ignoring and continuing
                    continue;
                };
                command_count += 1;

                if tx_rec.current_stage() > TransactionPoolStage::LocalPrepared {
                    // CASE: This will happen if output-only nodes send a prepare to input-involved nodes.
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Foreign LocalPrepare proposal ({}) received LOCAL_PREPARE for transaction {} but current transaction stage is {}. Ignoring.",
                        proposal,
                        tx_rec.id(),
                        tx_rec.current_stage()
                    );
                    continue;
                }

                if tx_rec.update_locked_epoch(proposal.to_locked_epoch()) {
                    debug!(
                        target: LOG_TARGET,
                        "🔒 Foreign proposal {} updated locked epoch for transaction {} to {}",
                        proposal,
                        tx_rec.id(),
                        proposal.epoch()
                    );
                } else {
                    debug!(
                        target: LOG_TARGET,
                        "Foreign proposal {} locked epoch for transaction {} remains at {}",
                        proposal,
                        tx_rec.id(),
                        tx_rec.locked_epoch().display()
                    );
                }

                let remote_decision = atom.decision;
                let local_decision = tx_rec.current_decision();
                if let Some(abort_reason) = remote_decision.abort_reason() &&
                    local_decision.is_commit()
                {
                    info!(
                        target: LOG_TARGET,
                        "⚠️ Foreign committee ABORT transaction {}. Update overall decision to ABORT. Local stage: {}, Leaf: {}",
                        tx_rec.id(), tx_rec.current_stage(), local_leaf
                    );

                    // Add an abort execution since we previously decided to commit
                    let exec =
                        TransactionExecution::abort(tx_rec.id(), RejectReason::ForeignShardGroupDecidedToAbort {
                            start_shard: foreign_shard_group.start().as_u32(),
                            end_shard: foreign_shard_group.end().as_u32(),
                            abort_reason,
                        });
                    tx_rec.set_local_decision(exec.decision());
                    proposed_block_change_set.add_transaction_execution(*tx_rec.id(), exec)?;
                }

                // Update the transaction record with any new information provided by this foreign block
                let Some(foreign_evidence) = atom.evidence.get(&foreign_shard_group) else {
                    return Err(ProposalValidationError::ForeignInvalidPledge {
                        block: foreign_proposal_as_leaf,
                        transaction_id: atom.id,
                        shard_group: foreign_shard_group,
                        details: "Foreign proposal did not contain evidence for its own shard group".to_string(),
                    }
                    .into());
                };

                let justify_qc_id = proposal
                    .get_justify_qc()
                    .map(calculate_qc_id_from_sidechain_qc)
                    .ok_or_else(|| {
                        HotStuffError::InvariantError(format!(
                            "Foreign proposal {} does not contain a justify QC for shard group {} - this should have \
                             been rejected by validations",
                            foreign_block_id, foreign_shard_group
                        ))
                    })?;

                tx_rec
                    .evidence_mut()
                    .add_shard_group(foreign_shard_group)
                    .update(foreign_evidence)
                    .set_prepare_qc(justify_qc_id);
                tx_rec.set_remote_decision(remote_decision);

                // CASE: local node has pledged a substate S to tx_1 and foreign node has locked the same substate S to
                // tx_2. If the local node has already prepared the transaction, we must abort tx_2. If
                // they have received our localprepare, they will abort tx_1 and we'll both abort.

                let inputs = tx_rec
                    .evidence()
                    .get(&local_committee_info.shard_group())
                    .map(|ev| ev.inputs().keys())
                    .into_iter()
                    .flatten();

                // If we have not yet prepared the transaction, we can wait to do that until this transaction is
                // finalised and there is no need to abort either. If we have already prepared the transaction, we
                // need to abort in this case since we've already pledged.
                if tx_rec.current_decision().is_commit() &&
                    let Some(conflicting_transaction_id) =
                        LockedSubstateValue::get_transaction_id_that_conflicts_with_write_locks(
                            substate_store.read_transaction(),
                            tx_rec.id(),
                            inputs,
                        )?
                {
                    // Determine if we're the only conflicting shard group. If so, we can ignore this conflict
                    // and wait for the conflicting transaction to be finalised before pledging, using the outputs
                    // of this transaction. If not, resolving the conflict is more
                    // complex/unsafe, and we need to abort.
                    let conflicting_tx = proposed_block_change_set.get_transaction_pool_record(
                        tx,
                        local_leaf,
                        &conflicting_transaction_id,
                    )?;
                    let has_conflicts = conflicting_tx
                        .evidence()
                        .iter()
                        .filter(|(sg, _)| **sg != local_committee_info.shard_group())
                        .any(|(sg, conflicting_evidence)| {
                            tx_rec.evidence().get(sg).is_some_and(|shard_ev| {
                                shard_ev.inputs().iter().any(|(id, e)| {
                                    // If the current transaction (tx_rec) has a strict input version, it will be
                                    // aborted later.
                                    match conflicting_evidence.inputs().get(id) {
                                        Some(Some(ev)) => {
                                            let conflicting_is_write = e.as_ref().is_none_or(|e| e.is_write);
                                            // If either are write, we ABORT
                                            conflicting_is_write || ev.is_write
                                        },
                                        Some(None) => {
                                            // We don't know the pledge, so we have to assume write
                                            // TODO: check if this can ever legitimately happen
                                            true
                                        },
                                        None => {
                                            // No matching input - OK
                                            false
                                        },
                                    }
                                })
                            })
                        });
                    if has_conflicts {
                        warn!(
                            target: LOG_TARGET,
                            "⚠️ Foreign proposal {} received for transaction {} but a conflicting transaction ({}) has already been prepared by this node. Abort.",
                            proposal,
                            tx_rec.id(),
                            conflicting_transaction_id
                        );

                        // Add an abort execution since we previously decided to commit
                        let exec = TransactionExecution::abort(tx_rec.id(), RejectReason::ForeignPledgeInputConflict);
                        tx_rec.set_local_decision(exec.decision());
                        proposed_block_change_set.add_transaction_execution(*tx_rec.id(), exec)?;
                    } else {
                        info!(
                            target: LOG_TARGET,
                            "Transaction {} conflicts with {} but does not conflict with any foreign pledges. No need to abort.",
                            tx_rec.id(),
                            conflicting_transaction_id
                        );
                    }
                }

                add_pledges(
                    &tx_rec,
                    foreign_proposal_as_leaf,
                    atom,
                    block_pledge,
                    foreign_shard_group,
                    local_committee_info.shard_group(),
                    proposed_block_change_set,
                    true,
                )?;

                // tx_rec.evidence().iter().for_each(|(addr, ev)| {
                //     let includes_local = local_committee_info.includes_substate_address(addr);
                //     log::error!(
                //         target: LOG_TARGET,
                //         "🐞 LOCALPREPARE EVIDENCE (l={}, f={}) {}: {}", includes_local, !includes_local, addr, ev
                //     );
                // });
                let local_shard_group = local_committee_info.shard_group();

                if tx_rec.current_stage().is_new() {
                    info!(
                        target: LOG_TARGET,
                        "🧩 FOREIGN PROPOSAL {foreign_shard_group}: (Initial sequence from LocalPrepare) Transaction is ready for Prepare({}, {}) Local Stage: {}",
                        tx_rec.id(),
                        tx_rec.current_decision(),
                        tx_rec.current_stage()
                    );
                    // If the transaction is New, we're waiting for all foreign pledges. Propose transaction once we
                    // have them.

                    // CASE: One foreign SG is involved in all inputs and executed the transaction, local SG is
                    // involved in the outputs
                    let is_ready = tx_rec.current_decision().is_abort() ||
                        // local_committee_info.includes_substate_id(&tx_rec.to_receipt_id().into()) ||
                        tx_rec.committee_involves_inputs(local_committee_info) ||
                        has_all_foreign_input_pledges(tx, &tx_rec, local_committee_info, proposed_block_change_set)?;

                    if is_ready {
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL {foreign_shard_group}: (Initial sequence from LocalPrepare) Transaction is ready for Prepare({}, {}) Local Stage: {}",
                            tx_rec.id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );
                        tx_rec.set_ready(true);
                        tx_rec.set_next_stage_and_readiness(TransactionPoolStage::New, local_shard_group)?;
                    } else {
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL {foreign_shard_group}: (Initial sequence from LocalPrepare) Transaction is NOT ready for Prepare({}, {}) Local Stage: {}",
                            tx_rec.id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );
                        // If foreign abort, we are ready to propose ABORT
                        if tx_rec.current_decision().is_abort() {
                            tx_rec.set_ready(true);
                        }
                    }
                } else if tx_rec.current_stage().is_local_prepared() &&
                    tx_rec.is_ready_for_pending_stage(local_shard_group)
                {
                    // If all shards are complete, and we've already received our LocalPrepared, we can set the
                    // LocalPrepared transaction as ready to propose AllPrepared. If we have not received
                    // the local LocalPrepared, the transition to AllPrepared will occur after we receive the local
                    // LocalPrepare proposal.
                    info!(
                        target: LOG_TARGET,
                        "🧩 FOREIGN PROPOSAL {foreign_shard_group}: Transaction is ready for propose AllPrepared({}, {}) Local Stage: {}",
                        tx_rec.id(),
                        tx_rec.current_decision(),
                        tx_rec.current_stage()
                    );

                    tx_rec.set_next_stage_and_readiness(TransactionPoolStage::LocalPrepared, local_shard_group)?;
                } else {
                    info!(
                        target: LOG_TARGET,
                        "🧩 FOREIGN PROPOSAL {foreign_shard_group}: Transaction is NOT ready for AllPrepared({}, {}) Local Stage: {}, \
                        All Justified: {}. Waiting for local proposal and/or additional foreign proposals for all other shard groups.",
                        tx_rec.id(),
                        tx_rec.current_decision(),
                        tx_rec.current_stage(),
                        tx_rec.evidence().all_input_shard_groups_prepared(local_shard_group)
                    );
                }

                proposed_block_change_set.set_next_transaction_update(tx_rec)?;
            },
            Command::LocalAccept(atom) => {
                if !atom.evidence.has(&local_committee_info.shard_group()) {
                    // Should not happen, since foreign shard groups should only send applicable commands
                    warn!(
                        target: LOG_TARGET,
                        "🧩❓️ FOREIGN PROPOSAL {foreign_shard_group}: Command: LocalAccept({}, {}), block: {} not relevant to local committee",
                        atom.id, atom.decision, foreign_block_id,
                    );
                    continue;
                }

                debug!(
                    target: LOG_TARGET,
                    "🧩 FOREIGN PROPOSAL {foreign_shard_group}: Command: LocalAccept({}, {}), block: {}",
                    atom.id, atom.decision, foreign_block_id,
                );

                let Some(mut tx_rec) = get_or_sequence_transaction(
                    tx,
                    local_committee_info.num_preshards(),
                    local_committee_info.num_committees(),
                    &atom.id,
                    local_leaf,
                    &foreign_block_id,
                    proposed_block_change_set,
                )?
                else {
                    // TODO: evaluate this case - None is returned if the transaction is already finalized, which could
                    // be a race condition however, gracefully handling an error is more correct
                    // than just ignoring and continuing
                    continue;
                };

                command_count += 1;

                if tx_rec.current_stage() > TransactionPoolStage::LocalAccepted {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Foreign proposal {} received LOCAL_ACCEPT for transaction {} but current transaction stage is {}. Ignoring.",
                        proposal,
                        tx_rec.id(),
                        tx_rec.current_stage(),
                    );
                    continue;
                }

                // We update this in the foreign Accept phase because we may be output-only and not yet performed the
                // prepare phase. (therefore locked_epoch is None)
                tx_rec.update_locked_epoch(proposal.to_locked_epoch());

                let remote_decision = atom.decision;
                let local_decision = tx_rec.current_decision();
                if let Some(abort_reason) = remote_decision.abort_reason() &&
                    local_decision.is_commit()
                {
                    info!(
                        target: LOG_TARGET,
                        "⚠️ Foreign {foreign_shard_group} ABORT {}. Update overall decision to ABORT. Local stage: {}, Leaf: {}",
                        tx_rec.id(), tx_rec.current_stage(), local_leaf
                    );
                    // Add an abort execution since we previously decided to commit
                    let exec =
                        TransactionExecution::abort(tx_rec.id(), RejectReason::ForeignShardGroupDecidedToAbort {
                            start_shard: foreign_shard_group.start().as_u32(),
                            end_shard: foreign_shard_group.end().as_u32(),
                            abort_reason,
                        });
                    tx_rec.set_local_decision(exec.decision());
                    proposed_block_change_set.add_transaction_execution(*tx_rec.id(), exec)?;
                }

                // Update the transaction record with any new information provided by this foreign block
                let Some(foreign_evidence) = atom.evidence.get(&foreign_shard_group) else {
                    return Err(ProposalValidationError::ForeignInvalidPledge {
                        block: foreign_proposal_as_leaf,
                        transaction_id: atom.id,
                        shard_group: foreign_shard_group,
                        details: "Foreign proposal did not contain evidence for its own shard group".to_string(),
                    }
                    .into());
                };
                let justify_qc_id = proposal
                    .get_justify_qc()
                    .map(calculate_qc_id_from_sidechain_qc)
                    .ok_or_else(|| {
                        HotStuffError::InvariantError(format!(
                            "Foreign proposal {} does not contain a justify QC for shard group {} - this should have \
                             already been validated",
                            foreign_block_id, foreign_shard_group
                        ))
                    })?;

                tx_rec
                    .evidence_mut()
                    .add_shard_group(foreign_shard_group)
                    .update(foreign_evidence)
                    .set_accept_qc(justify_qc_id);
                tx_rec.set_remote_decision(remote_decision);

                add_pledges(
                    &tx_rec,
                    foreign_proposal_as_leaf,
                    atom,
                    block_pledge,
                    foreign_shard_group,
                    local_committee_info.shard_group(),
                    proposed_block_change_set,
                    false,
                )?;

                let local_shard_group = local_committee_info.shard_group();

                if tx_rec.current_stage().is_new() {
                    // If the transaction is New, we're waiting for all foreign pledges. Propose transaction once we
                    // have them.
                    // CASE: Foreign SGs have pledged all inputs and executed the transaction, local SG is involved
                    // in the outputs
                    let is_ready = //local_committee_info.includes_substate_id(&tx_rec.to_receipt_id().into()) ||
                        tx_rec.committee_involves_inputs(local_committee_info) ||
                        has_all_foreign_input_pledges(tx, &tx_rec, local_committee_info, proposed_block_change_set)?;
                    if is_ready {
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL {foreign_shard_group}: (Initial sequence from LocalAccept) Transaction is ready for Prepare({}, {}) Local Stage: {}",
                            tx_rec.id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );

                        tx_rec.set_ready(true);
                        tx_rec.set_next_stage_and_readiness(TransactionPoolStage::New, local_shard_group)?;
                    } else {
                        info!(
                            target: LOG_TARGET,
                            "🧩 FOREIGN PROPOSAL {foreign_shard_group}: (Initial sequence from LocalAccept) Transaction is NOT ready for Prepare({}, {}) Local Stage: {}",
                            tx_rec.id(),
                            tx_rec.current_decision(),
                            tx_rec.current_stage()
                        );

                        // If foreign abort, we are ready to propose ABORT
                        if tx_rec.current_decision().is_abort() {
                            tx_rec.set_ready(true);
                        }
                    }
                } else if tx_rec.current_stage().is_local_prepared() &&
                    tx_rec.is_ready_for_pending_stage(local_shard_group)
                {
                    info!(
                        target: LOG_TARGET,
                        "🧩 FOREIGN PROPOSAL {foreign_shard_group}: Transaction is ready for propose ALL_PREPARED({}, {}) Local Stage: {}",
                        tx_rec.id(),
                        tx_rec.current_decision(),
                        tx_rec.current_stage()
                    );

                    // Set readiness according to the new evidence even if in LocalPrepared phase. We may get
                    // LocalPrepared foreign proposal after this.
                    tx_rec.set_next_stage_and_readiness(TransactionPoolStage::LocalPrepared, local_shard_group)?;
                } else if tx_rec.current_stage().is_local_accepted() &&
                    tx_rec.is_ready_for_pending_stage(local_shard_group)
                {
                    info!(
                        target: LOG_TARGET,
                        "🧩 FOREIGN PROPOSAL {foreign_shard_group}: Transaction is ready for propose ALL_ACCEPT({}, {}) Local Stage: {}",
                        tx_rec.id(),
                        tx_rec.current_decision(),
                        tx_rec.current_stage()
                    );

                    tx_rec.set_next_stage_and_readiness(TransactionPoolStage::LocalAccepted, local_shard_group)?;
                } else {
                    info!(
                        target: LOG_TARGET,
                        "🧩 FOREIGN PROPOSAL {foreign_shard_group}: Transaction is NOT ready for AllAccept({}, {}) Local Stage: {}, All Justified: {}. Waiting for local or foreign proposal.",
                        tx_rec.id(),
                        tx_rec.current_decision(),
                        tx_rec.current_stage(),
                        tx_rec.evidence().all_shard_groups_accepted()
                    );
                }
                // Add the next transaction update to the proposed block change set
                proposed_block_change_set.set_next_transaction_update(tx_rec)?;
            },
            // Should never receive this
            Command::EndEpoch(_) => {
                warn!(
                    target: LOG_TARGET,
                    "❓️ NEVER HAPPEN: Foreign proposal received {} contains an EndEpoch command. This is invalid behaviour but continuing anyway.",
                    proposal
                );
                continue;
            },
            // These are not included
            // TODO: validate that these are not included in the foreign proposal
            Command::AllAccept(_) | Command::SomeAccept(_) | Command::LocalOnly(_) | Command::ForeignProposal(_) => {
                // Disregard
                continue;
            },
        }
    }

    info!(
        target: LOG_TARGET,
        "🧩 FOREIGN PROPOSAL: Processed {} commands from foreign block {}",
        command_count,
        proposal
    );
    if command_count == 0 {
        warn!(
            target: LOG_TARGET,
            "⚠️ FOREIGN PROPOSAL: No commands were applicable for foreign block {}. Ignoring.",
            proposal
        );
    }

    Ok(())
}

fn add_pledges(
    transaction: &TransactionPoolRecord,
    foreign_block: LeafBlock,
    atom: &TransactionAtom,
    block_pledge: &BlockPledge,
    foreign_shard_group: ShardGroup,
    local_shard_group: ShardGroup,
    proposed_block_change_set: &mut ProposedBlockChangeSet,
    is_prepare_phase: bool,
) -> Result<(), HotStuffError> {
    let _timer = TraceTimer::info(LOG_TARGET, "validate_and_add_pledges");

    // Avoid iterating unless debug logs apply
    if log_enabled!(Level::Debug) {
        debug!(
            target: LOG_TARGET,
            "PLEDGES FOR TRANSACTION: {atom}",
        );
        if block_pledge.is_empty() {
            debug!(
                target: LOG_TARGET,
                "No pledges for transaction {}",
                atom.id
            );
        } else {
            debug!(
                target: LOG_TARGET,
                "FOREIGN PLEDGE {block_pledge}",
            );
        }
    }

    match atom.decision {
        Decision::Commit => {
            // Output pledges come straight from evidence
            let foreign_sg_evidence = transaction.evidence().get(&foreign_shard_group).ok_or_else(|| {
                // NEVER HAPPEN: we should already have checked this
                ProposalValidationError::ForeignInvalidPledge {
                    block: foreign_block,
                    transaction_id: atom.id,
                    shard_group: foreign_shard_group,
                    details: "Foreign proposal did not contain evidence for its own shard group".to_string(),
                }
            })?;
            let output_pledges = foreign_sg_evidence.output_pledge_iter().collect();
            proposed_block_change_set.add_foreign_pledges(transaction.id(), foreign_shard_group, output_pledges);

            let Some(pledges) = block_pledge.get_all_pledges_for_evidence(foreign_sg_evidence) else {
                if transaction.evidence().is_committee_output_only(foreign_shard_group) {
                    debug!(
                        target: LOG_TARGET,
                        "Foreign proposal for transaction {} stage: {} but the foreign shard group is only involved in outputs so no output pledge is expected.",
                        atom.id,
                        transaction.current_stage()
                    );
                    return Ok(());
                }
                // Accept phase: We only require pledges for inputs in the accept phase if the local shard group is
                // output-only. Otherwise, the pledging has already happened in the prepare
                // phase.
                if !is_prepare_phase && !transaction.evidence().is_committee_output_only(local_shard_group) {
                    debug!(
                        target: LOG_TARGET,
                        "Foreign proposal for transaction {} stage: {}. The local shard group is involved in inputs so pledges should already have been pledged in the prepare phase.",
                        atom.id,
                        transaction.current_stage()
                    );
                    return Ok(());
                }
                warn!(
                    target: LOG_TARGET,
                    "⚠️❌ Foreign proposal for transaction {} stage: {} but no pledges found in the block pledge. Evidence: {}",
                    atom.id,
                    transaction.current_stage(),
                    transaction.evidence()
                );

                return Err(HotStuffError::ForeignNodeOmittedTransactionPledges {
                    foreign_block,
                    transaction_id: atom.id,
                    is_prepare_phase,
                });
            };

            // If the foreign shard has committed the transaction, we can add the pledges to the transaction
            // record
            debug!(
                target: LOG_TARGET,
                "Adding {} foreign pledge(s) to transaction {}. Foreign shard group: {}. Pledges: {}",
                pledges.len(),
                atom.id,
                foreign_shard_group,
                pledges.display()
            );

            proposed_block_change_set.add_foreign_pledges(transaction.id(), foreign_shard_group, pledges);
        },
        Decision::Abort(reason) => {
            debug!(target: LOG_TARGET, "Transaction {} was ABORTED by foreign node ({}). No pledges expected.", atom.id, reason);
        },
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
        .filter(|(sg, _)| local_committee_info.shard_group() != **sg)
        .flat_map(|(_, ev)| ev.inputs());

    let current_pledges = proposed_block_change_set.get_foreign_pledges(tx_rec.id());

    for (id, data) in foreign_inputs {
        let Some(data) = data else {
            // Case: Foreign shard group evidence is not yet fully populated therefore we do not consider the input
            // pledged
            return Ok(false);
        };
        // Check the current block change set to see if the pledge is included
        if current_pledges
            .clone()
            .any(|pledge| pledge.satisfies_substate_and_version(id, data.version))
        {
            continue;
        }

        if tx.foreign_substate_pledges_exists_for_transaction_and_address(
            tx_rec.id(),
            SubstateAddress::from_substate_id(id, data.version),
        )? {
            continue;
        }
        debug!(
            target: LOG_TARGET,
            "Transaction {} is missing a pledge for input {}",
            tx_rec.id(),
            id
        );
        return Ok(false);
    }

    Ok(true)
}

fn get_or_sequence_transaction<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    num_preshards: NumPreshards,
    num_committees: u32,
    transaction_id: &TransactionId,
    local_leaf: &LeafBlock,
    foreign_block_id: &BlockId,
    proposed_block_change_set: &mut ProposedBlockChangeSet,
) -> Result<Option<TransactionPoolRecord>, HotStuffError> {
    match proposed_block_change_set
        .get_transaction_pool_record(tx, local_leaf, transaction_id)
        .optional()?
    {
        Some(tx_rec) => Ok(Some(tx_rec)),
        None => match TransactionRecord::get(tx, transaction_id).optional()? {
            Some(transaction) => {
                if transaction.is_finalized(tx)? {
                    info!(
                        target: LOG_TARGET,
                        "❓️Foreign proposal {} received for transaction {} but this transaction is already finalized.",
                        foreign_block_id,
                        transaction_id
                    );
                    return Ok(None);
                }
                info!(
                    target: LOG_TARGET,
                    "🧩 Foreign proposal {} received for transaction {} but it has yet to be sequenced.",
                    foreign_block_id,
                    transaction_id
                );

                // When sequencing the transaction for the first time it is very important to add initial evidence
                // since we use it to determine if we have all foreign pledges ready for execution.
                let initial_evidence = Evidence::from_inputs_and_outputs(
                    num_preshards,
                    num_committees,
                    transaction.transaction.all_inputs_iter(),
                    transaction.transaction.known_outputs_iter(),
                );

                let pool_tx =
                    proposed_block_change_set.sequence_new_transaction(transaction.transaction(), initial_evidence);
                Ok(Some(pool_tx))
            },
            None => {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ NEVER HAPPEN: Foreign proposal {} received for transaction {} but this transaction is unknown and not in the pool.",
                    foreign_block_id,
                    transaction_id
                );
                Ok(None)
            },
        },
    }
}

fn calculate_qc_id_from_sidechain_qc(qc: &tari_sidechain::QuorumCertificate) -> PcId {
    let signatures = qc
        .signatures
        .iter()
        .map(|s| ValidatorSignatureBytes {
            public_key: RistrettoPublicKeyBytes::from_bytes(s.public_key.as_bytes())
                .expect("invariant: public key bytes"),
            signature: SchnorrSignatureBytes::new(
                RistrettoPublicKeyBytes::from_bytes(s.signature.get_compressed_public_nonce().as_bytes())
                    .expect("invariant: nonce bytes"),
                Scalar32Bytes::from_bytes(s.signature.get_signature().as_bytes()).expect("invariant: signature bytes"),
            ),
        })
        .collect::<Vec<_>>();
    ProposalCertificate::calculate_id_from_parts(
        &qc.header_hash,
        &qc.parent_id.into_array().into(),
        &signatures,
        &qc.decision,
    )
}
