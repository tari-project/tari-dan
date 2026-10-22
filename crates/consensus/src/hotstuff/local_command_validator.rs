//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::num::NonZeroU64;

use log::*;
use tari_consensus_types::{Decision, LeafBlock};
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_engine_types::commit_result::{AbortReason, RejectReason};
use tari_ootle_common_types::{committee::CommitteeInfo, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    consensus_models::{
        Block,
        BlockTransactionExecution,
        InvalidEvidenceReason,
        LocalOnlyAtom,
        LockedEpoch,
        MultiShardAtom,
        NoVoteReason,
        TransactionExecution,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionRecord,
    },
};

use crate::{
    hotstuff::{
        abort_if_local_prepared_expired,
        abort_local_prepared_for_epoch_end,
        block_change_set::ProposedBlockChangeSet,
        error::HotStuffError,
        filter_diff_for_committee,
        local_pledged_inputs,
        on_ready_to_vote_on_local_block::check_ready_to_leave_stage,
        substate_store::{LockStatus, PendingSubstateStore},
        transaction_manager::{
            ConsensusTransactionManager,
            EvidenceOrExecution,
            LocalPreparedTransaction,
            PledgedTransaction,
            PreparedTransaction,
        },
    },
    tracing::TraceTimer,
    traits::{BlockTransactionExecutor, WriteableSubstateStore},
};

// Logs under the target of the handler that drives these checks, so operators filter one target for every no-vote.
const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::on_ready_to_vote_on_local_block";

/// Decides whether a replica can vote for each transaction command in a block proposed by its own committee.
pub(super) struct LocalCommandValidator<'a, TExecutor, TStateStore> {
    transaction_manager: &'a ConsensusTransactionManager<TExecutor, TStateStore>,
    local_validator_pk: &'a RistrettoPublicKey,
}

impl<'a, TExecutor, TStateStore> LocalCommandValidator<'a, TExecutor, TStateStore>
where
    TStateStore: StateStore,
    TExecutor: BlockTransactionExecutor<TStateStore>,
{
    pub fn new(
        transaction_manager: &'a ConsensusTransactionManager<TExecutor, TStateStore>,
        local_validator_pk: &'a RistrettoPublicKey,
    ) -> Self {
        Self {
            transaction_manager,
            local_validator_pk,
        }
    }

    #[allow(clippy::too_many_lines)]
    pub fn evaluate_local_only_command<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        block: &Block,
        atom: &LocalOnlyAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TTx>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
        total_leader_fee: &mut u64,
        total_exhaust_burn: &mut u128,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        let _timer = TraceTimer::info(LOG_TARGET, "Evaluate LocalOnly command");
        let Some(mut pool_tx) = proposed_block_change_set
            .get_transaction_pool_record(tx, substate_store.parent_chain(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(Some(NoVoteReason::TransactionNotInPool));
        };

        if !pool_tx.current_stage().is_new() {
            warn!(
                target: LOG_TARGET,
                "❌ Stage disagreement for tx {} in block {}. Leader proposed LocalOnly, local stage is {}",
                pool_tx.id(),
                block,
                pool_tx.current_stage(),
            );
            return Ok(Some(NoVoteReason::StageDisagreement {
                stage: pool_tx.current_stage(),
                expected: TransactionPoolStage::New,
            }));
        }
        let locked_epoch = block.to_locked_epoch();
        // LocalOnly so we can lock the epoch here
        pool_tx.update_locked_epoch(locked_epoch.clone());

        let prepared = self
            .transaction_manager
            .prepare(
                substate_store,
                local_committee_info,
                &pool_tx,
                block.as_leaf(),
                proposed_block_change_set,
            )
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        match prepared {
            PreparedTransaction::LocalOnly(local) => {
                match *local {
                    LocalPreparedTransaction::Accept { execution, .. } => {
                        pool_tx
                            .set_local_decision(execution.decision())
                            .set_transaction_fee(execution.transaction_fee())
                            .set_exhaust_burn(execution.exhaust_burn())
                            .set_evidence(execution.to_evidence(
                                local_committee_info.num_preshards(),
                                local_committee_info.num_committees(),
                            ));

                        info!(
                            target: LOG_TARGET,
                            "👨‍🔧 LocalOnly: Prepare for transaction {} ({}) in block {}",
                            pool_tx.id(),
                            pool_tx.current_decision(),
                            block,
                        );

                        // If the leader proposed to commit a transaction that we want to abort, we abstain from voting
                        if pool_tx.current_decision() != atom.decision {
                            // If we disagree with any local decision we abstain from voting
                            warn!(
                                target: LOG_TARGET,
                                "❌ Prepare decision disagreement for tx {} in block {}. Leader proposed {}, we decided {}",
                                pool_tx.id(),
                                block,
                                atom.decision,
                                pool_tx.current_decision()
                            );
                            return Ok(Some(NoVoteReason::DecisionDisagreement {
                                local: pool_tx.current_decision(),
                                remote: atom.decision,
                            }));
                        }

                        if pool_tx.transaction_fee() != atom.transaction_fee {
                            warn!(
                                target: LOG_TARGET,
                                "❌ LocalOnly transaction fee disagreement for block {}. Leader proposed {}, we calculated {}",
                                block,
                                atom.transaction_fee,
                                pool_tx.transaction_fee()
                            );
                            return Ok(Some(NoVoteReason::FeeDisagreement));
                        }

                        if pool_tx.current_decision().is_commit() {
                            if let Some(diff) = execution.result().finalize.any_accept() &&
                                let Err(err) = substate_store.put_diff(diff)
                            {
                                // An honest proposer skips a transaction whose diff fails to apply with a lock
                                // failure, so a block that sequences one must not be voted for. Any other store
                                // error is fatal.
                                let lock_err = err.ok_lock_failed()?;
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ LocalOnly transaction {} in block {} has a state diff that does not apply, which an honest proposer skips: {}. Not voting on block.",
                                    pool_tx.id(),
                                    block,
                                    lock_err,
                                );
                                return Ok(Some(NoVoteReason::ConflictingTransactionProposed {
                                    transaction_id: *pool_tx.id(),
                                    details: lock_err.to_string(),
                                }));
                            }

                            if atom.leader_fee.is_none() {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ Leader fee for tx {} is None for LocalOnly command in block {}",
                                    atom.id,
                                    block,
                                );
                                return Ok(Some(NoVoteReason::NoLeaderFee));
                            }

                            let calculated_leader_fee =
                                pool_tx.calculate_leader_fee(NonZeroU64::new(1).expect("1 > 0"));
                            if calculated_leader_fee != *atom.leader_fee.as_ref().expect("None already checked") {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ LocalOnly leader fee disagreement for block {}. Leader proposed {}, we calculated {}",
                                    block,
                                    atom.leader_fee.as_ref().expect("None already checked"),
                                    calculated_leader_fee
                                );

                                return Ok(Some(NoVoteReason::LeaderFeeDisagreement));
                            }

                            *total_leader_fee =
                                total_leader_fee
                                    .checked_add(calculated_leader_fee.fee())
                                    .ok_or_else(|| {
                                        HotStuffError::InvariantError(
                                            "Leader fee overflow when summing for block".to_string(),
                                        )
                                    })?;
                            // A LocalOnly transaction's evidence must contain exactly the local shard group, so its
                            // portion of the exhaust burn is the entire burn.
                            if pool_tx.evidence().num_shard_groups() != 1 {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ NO VOTE: LocalOnly transaction {} in block {} has evidence for {} shard groups",
                                    pool_tx.id(),
                                    block,
                                    pool_tx.evidence().num_shard_groups(),
                                );
                                return Ok(Some(NoVoteReason::LocalOnlyProposedForMultiShard));
                            }
                            let Some(exhaust_burn_portion) = pool_tx.evidence().exhaust_burn_portion(
                                calculated_leader_fee.exhaust_burn(),
                                local_committee_info.shard_group(),
                            ) else {
                                warn!(
                                    target: LOG_TARGET,
                                    "❌ NO VOTE: local shard group {} is not in the evidence for LocalOnly transaction {} in block {}",
                                    local_committee_info.shard_group(),
                                    pool_tx.id(),
                                    block,
                                );
                                return Ok(Some(NoVoteReason::InvalidEvidence {
                                    reason: InvalidEvidenceReason::MissingInvolvedShardGroup {
                                        shard_group: local_committee_info.shard_group(),
                                    },
                                }));
                            };
                            *total_exhaust_burn += u128::from(exhaust_burn_portion);
                        }

                        proposed_block_change_set.add_transaction_execution(*pool_tx.id(), execution)?;
                    },
                    LocalPreparedTransaction::EarlyAbort { execution, .. } => {
                        if atom.decision.is_commit() {
                            warn!(
                                target: LOG_TARGET,
                                "❌ Failed to lock inputs/outputs for transaction {} but leader proposed COMMIT. Not voting for block {}",
                                pool_tx.id(),
                                block,
                            );
                            return Ok(Some(NoVoteReason::DecisionDisagreement {
                                local: Decision::Abort(AbortReason::LockInputsOutputsFailed),
                                remote: Decision::Commit,
                            }));
                        }

                        // They want to ABORT a successfully executed transaction because of a lock conflict, which
                        // we also have.
                        info!(
                            target: LOG_TARGET,
                            "⚠️ Proposer chose to ABORT and we chose to ABORT due to lock conflict for transaction {} in block {}",
                            block,
                            pool_tx.id(),
                        );
                        pool_tx
                            .set_local_decision(execution.decision())
                            .set_transaction_fee(execution.transaction_fee())
                            .set_exhaust_burn(execution.exhaust_burn())
                            .set_evidence(execution.to_evidence(
                                local_committee_info.num_preshards(),
                                local_committee_info.num_committees(),
                            ));
                        proposed_block_change_set.add_transaction_execution(*pool_tx.id(), execution)?;
                    },
                }
            },
            PreparedTransaction::PledgeBudgetReached { .. } => {
                return Err(HotStuffError::InvariantError(format!(
                    "Transaction {} in block {} was deferred for the pledge budget, which only a proposing store has",
                    atom.id(),
                    block,
                )));
            },
            PreparedTransaction::InputsLocked { .. } => {
                return Err(HotStuffError::InvariantError(format!(
                    "Transaction {} in block {} was deferred for its locked inputs, which only a proposer checks",
                    atom.id(),
                    block,
                )));
            },
            PreparedTransaction::MultiShard(_) => {
                warn!(
                    target: LOG_TARGET,
                    "❌ transaction {} in block {} is not Local-Only but was proposed as LocalOnly",
                    atom.id(),
                    block,
                );
                return Ok(Some(NoVoteReason::LocalOnlyProposedForMultiShard));
            },
        }

        pool_tx.set_next_stage_and_readiness(TransactionPoolStage::LocalOnly, block.shard_group())?;
        proposed_block_change_set.set_next_transaction_update(pool_tx)?;
        Ok(None)
    }

    #[allow(clippy::too_many_lines)]
    fn initial_prepare_multishard<TTx: StateStoreReadTransaction>(
        &self,
        tx_rec: &mut TransactionPoolRecord,
        block: &Block,
        atom: &MultiShardAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TTx>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        let _timer = TraceTimer::info(LOG_TARGET, "Evaluate Prepare command");

        info!(
            target: LOG_TARGET,
            "👨‍🔧 PREPARE: Transaction {} in block {}",
            tx_rec.id(),
            block,
        );

        if !tx_rec.current_stage().is_new() {
            warn!(
                target: LOG_TARGET,
                "❌ Stage disagreement for tx {} in block {}. Leader proposed Prepare, local stage is {}",
                tx_rec.id(),
                block,
                tx_rec.current_stage(),
            );
            return Ok(Some(NoVoteReason::StageDisagreement {
                stage: tx_rec.current_stage(),
                expected: TransactionPoolStage::New,
            }));
        }

        // Prepare phase, ensure we set a locked_epoch if not already done
        tx_rec.update_locked_epoch(block.to_locked_epoch());

        // Foreign block could have already resulted in an ABORT execution
        let maybe_execution = proposed_block_change_set.take_transaction_execution(tx_rec.id());
        let prepared = if maybe_execution.as_ref().is_some_and(|e| e.decision().is_abort()) {
            let execution = maybe_execution.expect("is_some_and");
            info!(
                target: LOG_TARGET,
                "👨‍🔧 PREPARE: Transaction {} in block {} is already ABORTED by foreign block",
                tx_rec.id(),
                execution.block_id()
            );
            PreparedTransaction::new_multishard_executed(execution.into_transaction_execution(), LockStatus::new())
        } else {
            self.transaction_manager
                .prepare(
                    substate_store,
                    local_committee_info,
                    tx_rec,
                    block.as_leaf(),
                    proposed_block_change_set,
                )
                .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?
        };

        match prepared {
            PreparedTransaction::LocalOnly(_) => {
                warn!(
                    target: LOG_TARGET,
                    "❌ transaction {} in block {} is Local-Only but was proposed as Prepare",
                    atom.id(),
                    block,
                );
                return Ok(Some(NoVoteReason::MultiShardProposedForLocalOnly));
            },
            PreparedTransaction::PledgeBudgetReached { .. } => {
                return Err(HotStuffError::InvariantError(format!(
                    "Transaction {} in block {} was deferred for the pledge budget, which only a proposing store has",
                    atom.id(),
                    block,
                )));
            },
            PreparedTransaction::InputsLocked { .. } => {
                return Err(HotStuffError::InvariantError(format!(
                    "Transaction {} in block {} was deferred for its locked inputs, which only a proposer checks",
                    atom.id(),
                    block,
                )));
            },
            PreparedTransaction::MultiShard(multishard) => {
                // TODO: Because on_propose does not process foreign proposals before proposing, the decision abort
                // reason may mismatch (e.g. ExecutionFailure != ForeignShardGroupDecidedToAbort)
                // This is why we use is_same_outcome here. We should try to process foreign proposals in
                // on_propose
                if !multishard.current_decision().is_same_outcome(atom.decision) {
                    warn!(
                        target: LOG_TARGET,
                        "❌ Leader proposed {} for transaction {} but we decided {} in block {}",
                        atom.decision,
                        atom.id,
                        multishard.current_decision(),
                        block,
                    );
                    return Ok(Some(NoVoteReason::DecisionDisagreement {
                        local: multishard.current_decision(),
                        remote: atom.decision,
                    }));
                }

                // TODO: this is kinda hacky - we may not be involved in the transaction after ABORT execution,
                // but this would be invalid so we ensure that we are added to evidence. Ideally, we wouldn't
                // sequence this transaction at all - investigate.
                tx_rec
                    .evidence_mut()
                    .add_shard_group(local_committee_info.shard_group());

                match multishard.into_evidence_or_execution() {
                    EvidenceOrExecution::Execution { execution } => {
                        debug!(
                            target: LOG_TARGET,
                            "👨‍🔧 PREPARE: Transaction {} in block {} is executed",
                            tx_rec.id(),
                            block,
                        );
                        // CASE: All inputs are local and outputs are foreign (i.e. the transaction is
                        // executed), or we're output-only and have received all pledges.
                        tx_rec.update_from_execution(
                            local_committee_info.num_preshards(),
                            local_committee_info.num_committees(),
                            &execution,
                        );
                        if execution.decision().is_commit() {
                            let involves_inputs = tx_rec.evidence().has_inputs(local_committee_info.shard_group());
                            if !involves_inputs {
                                // Output only
                                let num_involved_shard_groups = tx_rec.evidence().num_shard_groups();
                                let involved = NonZeroU64::new(num_involved_shard_groups as u64).ok_or_else(|| {
                                    HotStuffError::InvariantError("Number of involved shard groups is 0".to_string())
                                })?;
                                let leader_fee = tx_rec.calculate_leader_fee(involved);
                                tx_rec.set_leader_fee(leader_fee);
                            }
                        }
                        proposed_block_change_set.add_transaction_execution(*tx_rec.id(), *execution)?;
                    },
                    EvidenceOrExecution::Evidence { evidence } => {
                        debug!(
                            target: LOG_TARGET,
                            "👨‍🔧 PREPARE: Transaction {} in block {} is not executed. Using partial evidence.",
                            tx_rec.id(),
                            block,
                        );
                        // CASE: All local inputs were resolved. We need to continue with consensus to get the
                        // foreign inputs/outputs.
                        tx_rec.set_local_decision(Decision::Commit);
                        // Set partial evidence for local inputs using what we know.
                        tx_rec.merge_evidence(evidence);
                        // tx_rec epoch will be locked by foreign proposal(s) when we get it
                    },
                }
            },
        }

        Ok(None)
    }

    pub fn evaluate_local_prepare_command<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        block: &Block,
        atom: &MultiShardAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TTx>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        let Some(mut tx_rec) = proposed_block_change_set
            .get_transaction_pool_record(tx, substate_store.parent_chain(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(Some(NoVoteReason::TransactionNotInPool));
        };

        if !tx_rec.current_stage().is_new() {
            warn!(
                target: LOG_TARGET,
                "{} ❌ LocalPrepare Stage disagreement in block {} for transaction {}. Leader proposed LocalPrepare, but local stage is {}",
                self.local_validator_pk,
                block,
                tx_rec.id(),
                tx_rec.current_stage()
            );
            return Ok(Some(NoVoteReason::StageDisagreement {
                expected: TransactionPoolStage::New,
                stage: tx_rec.current_stage(),
            }));
        }

        if let Some(reason) = self.initial_prepare_multishard(
            &mut tx_rec,
            block,
            atom,
            local_committee_info,
            substate_store,
            proposed_block_change_set,
        )? {
            return Ok(Some(reason));
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ LocalPrepared transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(Some(NoVoteReason::FeeDisagreement));
        }

        if atom.evidence.get(&local_committee_info.shard_group()) !=
            tx_rec.evidence().get(&local_committee_info.shard_group())
        {
            warn!(
                target: LOG_TARGET,
                "❌ LocalPrepared evidence disagreement tx {} in block {}. Leader proposed {}, local {}",
                tx_rec.id(),
                block,
                atom.evidence,
                tx_rec.evidence()
            );
            return Ok(Some(NoVoteReason::InvalidEvidence {
                reason: InvalidEvidenceReason::MismatchedEvidence,
            }));
        }

        tx_rec.set_next_stage_and_readiness(TransactionPoolStage::LocalPrepared, block.shard_group())?;
        proposed_block_change_set.set_next_transaction_update(tx_rec)?;

        Ok(None)
    }

    #[allow(clippy::too_many_lines)]
    pub fn evaluate_local_accept_command<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        block: &Block,
        atom: &MultiShardAtom,
        local_committee_info: &CommitteeInfo,
        next_epoch_changes_layout: bool,
        substate_store: &mut PendingSubstateStore<TTx>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        let Some(mut tx_rec) = proposed_block_change_set
            .get_transaction_pool_record(tx, substate_store.parent_chain(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(Some(NoVoteReason::TransactionNotInPool));
        };

        let forced_abort = abort_if_local_prepared_expired(&mut tx_rec, block.epoch()).or_else(|| {
            (atom.decision.is_abort() && next_epoch_changes_layout)
                .then(|| abort_local_prepared_for_epoch_end(&mut tx_rec))
                .flatten()
        });
        if let Some(execution) = forced_abort {
            proposed_block_change_set.add_transaction_execution(*tx_rec.id(), execution)?;
        }

        if tx_rec.current_stage().is_new() {
            // CASE: This was sequenced immediately as LocalAccept, which can only mean either we are aborting or we are
            // an output-only Shard Group
            if let Some(reason) = self.initial_prepare_multishard(
                &mut tx_rec,
                block,
                atom,
                local_committee_info,
                substate_store,
                proposed_block_change_set,
            )? {
                return Ok(Some(reason));
            }
            if !tx_rec.current_decision().is_abort() &&
                !tx_rec
                    .evidence()
                    .is_committee_output_only(local_committee_info.shard_group())
            {
                warn!(
                    target: LOG_TARGET,
                    "❌ LocalAccept: transaction {} in block {} is not output-only",
                    tx_rec.id(),
                    block,
                );
                return Ok(Some(NoVoteReason::OutputOnlyDisagreement {
                    transaction_id: *tx_rec.id(),
                    shard_group: local_committee_info.shard_group(),
                    command: "LocalAccept",
                    stage: tx_rec.current_stage(),
                }));
            }
        } else if tx_rec.current_decision().is_commit() {
            // CASE: We are in the LocalPrepared stage, and want to proceed to LocalAccept

            if !tx_rec.current_stage().is_local_prepared() {
                warn!(
                    target: LOG_TARGET,
                    "❌ LocalAccept: transaction {} in block {} is not in COMMITTED LocalPrepared stage (committed stage: {}, current stage: {})",
                    tx_rec.id(),
                    block,
                    tx_rec.committed_stage(),
                    tx_rec.current_stage(),
                );
                return Ok(Some(NoVoteReason::StageDisagreement {
                    expected: TransactionPoolStage::LocalPrepared,
                    stage: tx_rec.current_stage(),
                }));
            }

            // Readiness is evaluated before execution, which may change the decision and with it the readiness
            // condition
            if let Some(reason) = check_ready_to_leave_stage(&tx_rec, block, "LocalAccept") {
                return Ok(Some(reason));
            }

            let transaction = tx_rec.get_transaction(tx)?;
            if !transaction.has_all_required_input_pledges(tx, &block.as_leaf(), local_committee_info)? {
                warn!(
                    target: LOG_TARGET,
                    "❌ NO VOTE AllPrepare: transaction {} in block {} has not received all foreign input pledges",
                    tx_rec.id(),
                    block,
                );
                return Ok(Some(NoVoteReason::NotAllForeignInputPledges));
            }
            let transaction_id = *tx_rec.id();

            let locked_epoch = tx_rec.locked_epoch().ok_or_else(|| {
                HotStuffError::InvariantError(format!(
                    "Locked epoch not set for transaction {} in LocalAccept stage",
                    tx_rec.id()
                ))
            })?;
            let execution = self.execute_transaction(
                substate_store,
                local_committee_info,
                block.as_leaf(),
                locked_epoch.clone(),
                transaction,
                proposed_block_change_set,
            )?;
            let execution = execution.into_transaction_execution();

            if execution.decision().is_commit() {
                // Counted for the block pledge check once every command is evaluated. Preparing the transaction
                // bounded what its inputs alone may pledge.
                substate_store.pledge_inputs(
                    local_pledged_inputs(&tx_rec, local_committee_info.shard_group()),
                    usize::MAX,
                )?;
            }

            // TODO: can we modify input locks at this point? For multi-shard input transactions, we locked all inputs
            // as Write due to lack of information. We now know what locks are necessary, and this
            // block has the correct evidence so this should be fine.
            tx_rec.update_from_execution(
                local_committee_info.num_preshards(),
                local_committee_info.num_committees(),
                &execution,
            );

            if execution.decision().is_commit() {
                // Lock all local outputs
                let local_outputs = execution
                    .resulting_outputs()
                    .iter()
                    .filter(|o| local_committee_info.includes_substate_id(o.substate_id()));
                let lock_status = substate_store.try_lock_all(*tx_rec.id(), local_outputs, false)?;
                if let Some(err) = lock_status.failures().first() {
                    if atom.decision.is_commit() {
                        // If we disagree with any local decision we abstain from voting
                        warn!(
                            target: LOG_TARGET,
                            "❌ NO VOTE LocalAccept: Lock failure: {} but leader decided COMMIT for tx {} in block {}. Leader proposed COMMIT, we decided ABORT",
                            err,
                            tx_rec.id(),
                            block,
                        );
                        return Ok(Some(NoVoteReason::DecisionDisagreement {
                            local: Decision::Abort(AbortReason::LockOutputsFailed),
                            remote: Decision::Commit,
                        }));
                    }

                    info!(
                        target: LOG_TARGET,
                        "⚠️ Failed to lock outputs for transaction {} in block {}. Error: {}",
                        tx_rec.id(),
                        block,
                        err
                    );

                    let execution = TransactionExecution::abort(
                        &transaction_id,
                        RejectReason::FailedToLockOutputs(err.to_string()),
                    );

                    tx_rec
                        .set_local_decision(execution.decision())
                        .set_transaction_fee(0)
                        .set_exhaust_burn(0)
                        .no_leader_fee()
                        .set_next_stage_and_readiness(TransactionPoolStage::LocalAccepted, block.shard_group())?;

                    proposed_block_change_set
                        .add_transaction_execution(*tx_rec.id(), execution)?
                        .set_next_transaction_update(tx_rec)?;

                    return Ok(None);
                }
            }

            info!(
                target: LOG_TARGET,
                "👨‍🔧 LocalAccept: Executed transaction {} in block {} with decision {}",
                tx_rec.id(),
                block,
                execution.decision()
            );
            proposed_block_change_set.add_transaction_execution(*tx_rec.id(), execution)?;
        } else {
            // CASE: We are in the LocalPrepared stage and have decided to ABORT. There is nothing to execute.
            if !tx_rec.current_stage().is_local_prepared() {
                warn!(
                    target: LOG_TARGET,
                    "❌ LocalAccept: ABORTED transaction {} in block {} is not in LocalPrepared stage (committed stage: {}, current stage: {})",
                    tx_rec.id(),
                    block,
                    tx_rec.committed_stage(),
                    tx_rec.current_stage(),
                );
                return Ok(Some(NoVoteReason::StageDisagreement {
                    expected: TransactionPoolStage::LocalPrepared,
                    stage: tx_rec.current_stage(),
                }));
            }

            if let Some(reason) = check_ready_to_leave_stage(&tx_rec, block, "LocalAccept") {
                return Ok(Some(reason));
            }
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE LocalAccept: transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(Some(NoVoteReason::FeeDisagreement));
        }

        if tx_rec.current_decision().is_commit() {
            let Some(ref leader_fee) = atom.leader_fee else {
                warn!(
                    target: LOG_TARGET,
                    "❌ NO VOTE: Leader fee in tx {} not set for LocalAccept command in block {}",
                    atom.id,
                    block,
                );
                return Ok(Some(NoVoteReason::NoLeaderFee));
            };

            // Check the leader fee in the local accept phase. The fee only applied (is added to the block fee) for
            // AllAccept
            let num_involved_shard_groups = tx_rec.evidence().num_shard_groups();
            let involved = NonZeroU64::new(num_involved_shard_groups as u64)
                .ok_or_else(|| HotStuffError::InvariantError("Number of involved shard groups is 0".to_string()))?;
            let calculated_leader_fee = tx_rec.calculate_leader_fee(involved);
            if calculated_leader_fee != *leader_fee {
                warn!(
                    target: LOG_TARGET,
                    "❌ NO VOTE: LocalAccept leader fee disagreement for block {}. Leader proposed {}, we calculated {}",
                    block,
                    atom.leader_fee.as_ref().expect("None already checked"),
                    calculated_leader_fee
                );

                return Ok(Some(NoVoteReason::LeaderFeeDisagreement));
            }

            tx_rec.set_leader_fee(calculated_leader_fee);
        } else if atom.leader_fee.is_some() {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Leader fee in tx {} is set for LocalAccept ABORT command in block {}",
                atom.id,
                block,
            );
            return Ok(Some(NoVoteReason::LeaderFeeDisagreement));
        } else {
            // Ok
        }

        // on_propose does not process foreign proposals, so the QC evidence may not match the evidence here.
        // We only check that the input/output pledges match
        if !tx_rec.evidence().eq_pledges(&atom.evidence) {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Evidence mismatch for LocalAccept transaction {} in block {}. Leader proposed evidence {}, but we calculated {}",
                tx_rec.id(),
                block,
                atom.evidence,
                tx_rec.evidence()
            );
            return Ok(Some(NoVoteReason::InvalidEvidence {
                reason: InvalidEvidenceReason::MismatchedEvidence,
            }));
        }

        tx_rec.set_next_stage_and_readiness(TransactionPoolStage::LocalAccepted, block.shard_group())?;
        proposed_block_change_set.set_next_transaction_update(tx_rec)?;

        Ok(None)
    }

    #[allow(clippy::too_many_lines)]
    pub fn evaluate_all_accept_command<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        block: &Block,
        atom: &MultiShardAtom,
        local_committee_info: &CommitteeInfo,
        substate_store: &mut PendingSubstateStore<TTx>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
        total_leader_fee: &mut u64,
        total_exhaust_burn: &mut u128,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        if atom.decision.is_abort() {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: AllAccept command received for block {} but requires that the transaction is COMMIT",
                block.id(),
            );
            return Ok(Some(NoVoteReason::AllAcceptMustBeCommit {
                transaction_id: atom.id,
                block_id: *block.id(),
            }));
        }

        let Some(mut tx_rec) = proposed_block_change_set
            .get_transaction_pool_record(tx, substate_store.parent_chain(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ NO VOTE: Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(Some(NoVoteReason::TransactionNotInPool));
        };

        if !tx_rec.current_stage().is_local_accepted() {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: AllAccept Stage disagreement in block {} for transaction {}. Leader proposed AllAccept, but local stage is {}",
                block,
                tx_rec.id(),
                tx_rec.current_stage()
            );
            return Ok(Some(NoVoteReason::StageDisagreement {
                expected: TransactionPoolStage::LocalAccepted,
                stage: tx_rec.current_stage(),
            }));
        }

        if tx_rec.current_decision().is_abort() {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: AllAccept decision disagreement for transaction {} in block {}. Leader proposed COMMIT, we decided ABORT",
                tx_rec.id(),
                block,
            );
            return Ok(Some(NoVoteReason::DecisionDisagreement {
                local: atom.decision,
                remote: Decision::Commit,
            }));
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: AllAccept transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(Some(NoVoteReason::FeeDisagreement));
        }

        let Some(ref leader_fee) = atom.leader_fee else {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Leader fee in tx {} not set for AllAccept command in block {}",
                atom.id,
                block,
            );
            return Ok(Some(NoVoteReason::NoLeaderFee));
        };

        let local_leader_fee = tx_rec.leader_fee().ok_or_else(|| {
            HotStuffError::InvariantError(format!(
                "evaluate_all_accept_command: Transaction {} has COMMIT decision and is at LocalAccepted stage but \
                 leader fee is missing",
                tx_rec.id()
            ))
        })?;

        if local_leader_fee != leader_fee {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: Leader fee disagreement for tx {} in block {}. Leader proposed {}, we calculated {}",
                atom.id,
                block,
                leader_fee,
                local_leader_fee
            );
            return Ok(Some(NoVoteReason::LeaderFeeDisagreement));
        }

        if let Some(reason) = check_ready_to_leave_stage(&tx_rec, block, "AllAccept") {
            return Ok(Some(reason));
        }

        if !tx_rec.has_all_required_foreign_pledges(tx, local_committee_info)? {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: AllAccept disagreement for transaction {} in block {}. Leader proposed that all foreign pledges have been received but locally this is not the case",
                tx_rec.id(),
                block,
            );
            return Ok(Some(NoVoteReason::NotAllForeignInputPledges));
        }

        if *tx_rec.evidence() != atom.evidence {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: AllAccept disagreement for transaction {} in block {}. Leader proposed evidence {}, but we calculated {}",
                tx_rec.id(),
                block,
                atom.evidence,
                tx_rec.evidence()
            );
            return Ok(Some(NoVoteReason::InvalidEvidence {
                reason: InvalidEvidenceReason::MismatchedEvidence,
            }));
        }

        let execution = BlockTransactionExecution::get_pending_for_block(tx, tx_rec.id(), &block.as_leaf())
            .optional()?
            .ok_or_else(|| {
                HotStuffError::InvariantError(format!(
                    "evaluate_all_accept_command: Transaction {} has COMMIT decision but execution is missing",
                    tx_rec.id()
                ))
            })?;

        let diff = execution.result().finalize.any_accept().ok_or_else(|| {
            HotStuffError::InvariantError(format!(
                "evaluate_local_accept_command: Transaction {} has COMMIT decision but execution failed when proposing",
                tx_rec.id(),
            ))
        })?;

        *total_leader_fee = total_leader_fee
            .checked_add(leader_fee.fee())
            .ok_or_else(|| HotStuffError::InvariantError("Leader fee overflow when summing for block".to_string()))?;
        // Compute the portion from the local record's evidence: its key order is locally maintained (sorted), whereas
        // the atom's wire-decoded key order is not consensus-checked (evidence equality is order-independent).
        let Some(exhaust_burn_portion) = tx_rec
            .evidence()
            .exhaust_burn_portion(leader_fee.exhaust_burn(), local_committee_info.shard_group())
        else {
            warn!(
                target: LOG_TARGET,
                "❌ NO VOTE: local shard group {} is not in the evidence for transaction {} in block {}",
                local_committee_info.shard_group(),
                atom.id(),
                block,
            );
            return Ok(Some(NoVoteReason::InvalidEvidence {
                reason: InvalidEvidenceReason::MissingInvolvedShardGroup {
                    shard_group: local_committee_info.shard_group(),
                },
            }));
        };
        *total_exhaust_burn += u128::from(exhaust_burn_portion);

        if let Err(err) = substate_store.put_diff(&filter_diff_for_committee(local_committee_info, diff)) {
            // An honest proposer skips a transaction whose diff fails to apply with a lock failure, so a block that
            // sequences one must not be voted for. Any other store error is fatal.
            let lock_err = err.ok_lock_failed()?;
            warn!(
                target: LOG_TARGET,
                "❌ AllAccept transaction {} in block {} has a state diff that does not apply, which an honest proposer skips: {}. Not voting on block.",
                tx_rec.id(),
                block,
                lock_err,
            );
            return Ok(Some(NoVoteReason::ConflictingTransactionProposed {
                transaction_id: *tx_rec.id(),
                details: lock_err.to_string(),
            }));
        }

        tx_rec.set_next_stage_and_readiness(TransactionPoolStage::AllAccepted, block.shard_group())?;
        proposed_block_change_set.set_next_transaction_update(tx_rec)?;

        Ok(None)
    }

    pub fn evaluate_some_accept_command<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        block: &Block,
        atom: &MultiShardAtom,
        substate_store: &PendingSubstateStore<TTx>,
        proposed_block_change_set: &mut ProposedBlockChangeSet,
    ) -> Result<Option<NoVoteReason>, HotStuffError> {
        if atom.decision.is_commit() {
            warn!(
                target: LOG_TARGET,
                "❌ SomeAccept command received for block {} but requires that the atom is ABORT",
                block.id(),
            );
            return Ok(Some(NoVoteReason::SomeAcceptAtomMustBeAbort {
                transaction_id: atom.id,
                block_id: *block.id(),
            }));
        }

        let Some(mut tx_rec) = proposed_block_change_set
            .get_transaction_pool_record(tx, substate_store.parent_chain(), atom.id())
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "⚠️ Local proposal received ({}) for transaction {} which is not in the pool. This is likely a previous transaction that has been re-proposed. Not voting on block.",
                block,
                atom.id(),
            );
            return Ok(Some(NoVoteReason::TransactionNotInPool));
        };

        if !tx_rec.current_stage().is_local_accepted() {
            warn!(
                target: LOG_TARGET,
                "{} ❌ Stage disagreement in block {} for transaction {}. Leader proposed SomeAccept, but local stage is {}",
                self.local_validator_pk,
                block,
                tx_rec.id(),
                tx_rec.current_stage()
            );
            return Ok(Some(NoVoteReason::StageDisagreement {
                expected: TransactionPoolStage::LocalAccepted,
                stage: tx_rec.current_stage(),
            }));
        }

        // We check that the leader decision is the same as our local decision (this will change to ABORT once we've
        // received the foreign LocalAccept).
        if tx_rec.current_decision().is_commit() {
            warn!(
                target: LOG_TARGET,
                "❌ SomeAccept decision disagreement for transaction {} in block {}. Leader proposed ABORT, we decided COMMIT",
                tx_rec.id(),
                block,
            );
            return Ok(Some(NoVoteReason::DecisionDisagreement {
                local: Decision::Commit,
                remote: atom.decision,
            }));
        }

        if let Some(reason) = check_ready_to_leave_stage(&tx_rec, block, "SomeAccept") {
            return Ok(Some(reason));
        }

        if tx_rec.transaction_fee() != atom.transaction_fee {
            warn!(
                target: LOG_TARGET,
                "❌ SomeAccept transaction fee disagreement tx {} in block {}. Leader proposed {}, we calculated {}",
                tx_rec.id(),
                block,
                atom.transaction_fee,
                tx_rec.transaction_fee()
            );
            return Ok(Some(NoVoteReason::FeeDisagreement));
        }

        tx_rec.set_next_stage_and_readiness(TransactionPoolStage::SomeAccepted, block.shard_group())?;
        proposed_block_change_set.set_next_transaction_update(tx_rec)?;

        Ok(None)
    }

    fn execute_transaction<TTx: StateStoreReadTransaction>(
        &self,
        substate_store: &PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        block: LeafBlock,
        locked_epoch: LockedEpoch,
        transaction: TransactionRecord,
        change_set: &ProposedBlockChangeSet,
    ) -> Result<BlockTransactionExecution, HotStuffError> {
        info!(
            target: LOG_TARGET,
            "👨‍🔧 DECIDE: Executing transaction {} in block {}",
            transaction.id(),
            block,
        );
        let tx = substate_store.read_transaction();
        // Might have been executed already in on propose
        if let Some(execution) =
            BlockTransactionExecution::get_pending_for_block(tx, transaction.id(), &block).optional()?
        {
            return Ok(execution);
        }

        let mut pledged = PledgedTransaction::load_pledges(tx, &block, transaction)?;
        let transaction_id = *pledged.id();
        pledged
            .foreign_pledges
            .extend(change_set.get_foreign_pledges(&transaction_id).cloned());

        let execution = self
            .transaction_manager
            .execute(substate_store, local_committee_info, locked_epoch, block.epoch, pledged)
            .map_err(|e| HotStuffError::TransactionExecutorError(e.to_string()))?;

        Ok(execution.for_block(block, transaction_id))
    }
}

#[cfg(test)]
mod tests;
