//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, marker::PhantomData};

use indexmap::{IndexMap, IndexSet};
use log::*;
use tari_consensus_types::{Decision, LeafBlock};
use tari_engine_types::{
    commit_result::{AbortReason, RejectReason},
    substate::{Substate, SubstateId},
};
use tari_ootle_common_types::{
    Epoch,
    LockIntent,
    SubstateRequirement,
    SubstateRequirementRef,
    committee::CommitteeInfo,
    optional::{IsNotFoundError, Optional},
};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    consensus_models::{
        BlockTransactionExecution,
        Evidence,
        LockedEpoch,
        SubstateRequirementLockIntent,
        TransactionExecution,
        TransactionPoolRecord,
        TransactionRecord,
    },
};
use tari_ootle_transaction::{Transaction, TransactionId};

use super::{PledgedTransaction, PreparedTransaction};
use crate::{
    hotstuff::{
        block_change_set::ProposedBlockChangeSet,
        substate_store::{LockStatus, PendingSubstateStore, SubstateStoreError},
    },
    tracing::TraceTimer,
    traits::{BlockTransactionExecutor, BlockTransactionExecutorError},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::block_transaction_executor";

#[derive(Debug, Clone)]
pub struct ConsensusTransactionManager<TExecutor, TStateStore> {
    executor: TExecutor,
    max_transaction_validity_epochs: u64,
    _store: PhantomData<TStateStore>,
}

impl<TStateStore: StateStore, TExecutor: BlockTransactionExecutor<TStateStore>>
    ConsensusTransactionManager<TExecutor, TStateStore>
{
    pub fn new(executor: TExecutor, max_transaction_validity_epochs: u64) -> Self {
        Self {
            executor,
            max_transaction_validity_epochs,
            _store: PhantomData,
        }
    }

    /// The abort a transaction earns for declaring a window outside the epoch it was pinned to, or
    /// `None` if the window is acceptable.
    ///
    /// Both rules are evaluated against the pinned epoch — agreed across shard groups before
    /// execution — so every node reaches the same verdict regardless of how far its own view had
    /// lagged when the transaction was admitted. An out-of-window transaction is therefore sequenced
    /// as an abort, which all shard groups adopt, rather than being dropped by whichever group
    /// happened to be behind.
    fn window_abort_reason(&self, pinned_epoch: Epoch, max_epoch: Epoch) -> Option<AbortReason> {
        if pinned_epoch > max_epoch {
            return Some(AbortReason::EpochExpired);
        }
        let latest_permitted = Epoch(
            pinned_epoch
                .as_u64()
                .saturating_add(self.max_transaction_validity_epochs),
        );
        if max_epoch > latest_permitted {
            return Some(AbortReason::ValidityWindowTooLong);
        }
        None
    }

    pub fn prepare<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        pool_tx: &TransactionPoolRecord,
        block: LeafBlock,
        change_set: &ProposedBlockChangeSet,
    ) -> Result<PreparedTransaction, BlockTransactionExecutorError> {
        let _timer = TraceTimer::info(LOG_TARGET, "prepare");
        let transaction = pool_tx.get_transaction(store.read_transaction())?;
        let transaction_id = *transaction.id();

        if let Some(reason) = pool_tx.current_decision().abort_reason() {
            // CASE: A foreign proposal caused this transaction to be sequenced and immediately aborted
            warn!(target: LOG_TARGET, "⚠️ PREPARE: Transaction {transaction_id} is ABORTED before prepare: {reason}");
            let execution = self
                .fetch_execution(store, &transaction_id, &block)?
                .unwrap_or_else(|| TransactionExecution::abort(&transaction_id, RejectReason::Abort { reason }));
            return Ok(PreparedTransaction::new_multishard_executed(
                execution,
                LockStatus::new(),
            ));
        }
        let execution_epoch = pool_tx.locked_epoch().ok_or_else(|| {
            BlockTransactionExecutorError::InvariantError(format!(
                "Pool transaction {} prepare without locked epoch",
                transaction_id
            ))
        })?;

        let resolved_inputs = self.resolve_inputs(store, &transaction, local_committee_info)?;

        match resolved_inputs {
            ResolvedTransactionInputs::OnlyLocalInputs { local_versions } => {
                // Because we have only local inputs, this shard group unilaterally decides the locked epoch (though
                // output shardgroups still may abort)
                self.prepare_local_input_transaction(
                    store,
                    local_committee_info,
                    &transaction,
                    local_versions,
                    block,
                    execution_epoch.clone(),
                )
            },
            ResolvedTransactionInputs::LocalAndForeignInputs {
                local_versions,
                non_local_inputs,
            } => self.prepare_multishard_involved_transaction(
                store,
                local_committee_info,
                &transaction,
                local_versions,
                non_local_inputs,
                block,
            ),
            ResolvedTransactionInputs::OnlyForeignInputs { non_local_inputs } => self
                .prepare_multishard_output_only_transaction(
                    store,
                    local_committee_info,
                    &transaction,
                    non_local_inputs,
                    block,
                    change_set,
                    execution_epoch.clone(),
                ),
            ResolvedTransactionInputs::OnlyTombstones => {
                // Because we have only local inputs, this shard group unilaterally decides the locked epoch (though
                // output shards may abort) TODO: the pool record needs to be updated after this
                // function returns by the caller, which is not ideal
                self.prepare_local_input_transaction(
                    store,
                    local_committee_info,
                    &transaction,
                    IndexMap::new(),
                    block,
                    execution_epoch.clone(),
                )
            },
            ResolvedTransactionInputs::OneOrMoreLocalInputsNotFound { is_local_only, err } => {
                // Currently, this message will differ depending on which involved shard is asked.
                // e.g. local nodes will say "failed to lock inputs", foreign nodes will say "foreign shard abort"
                let execution =
                    TransactionExecution::abort(&transaction_id, RejectReason::SubstateNotFound(err.to_string()));
                if is_local_only {
                    warn!(target: LOG_TARGET, "⚠️ PREPARE: OneOrMoreInputsNotFound transaction {} local ABORT", transaction_id);
                    Ok(PreparedTransaction::new_local_early_abort(execution))
                } else {
                    warn!(target: LOG_TARGET, "⚠️ PREPARE: OneOrMoreLocalInputsNotFound transaction {} foreign ABORT", transaction_id);
                    Ok(PreparedTransaction::new_multishard_executed(
                        execution,
                        LockStatus::default(),
                    ))
                }
            },
        }
    }

    fn resolve_local_versions<'a, TTx: StateStoreReadTransaction>(
        &self,
        store: &PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        transaction: &'a Transaction,
    ) -> Result<ResolvedInputs<'a>, BlockTransactionExecutorError> {
        let mut local_inputs = IndexMap::with_capacity(transaction.num_inputs());

        let mut foreign_inputs = IndexSet::new();
        for input in transaction.all_inputs_iter() {
            if !local_committee_info.includes_substate_id(input.substate_id()) {
                foreign_inputs.insert(input);
                continue;
            }

            match input.version() {
                Some(version) => {
                    let id = input.with_version(version);
                    store.lock_assert_is_up(id)?;
                    debug!(target: LOG_TARGET, "Resolved LOCAL substate: {id}");
                    local_inputs.insert(input, version);
                },
                None => {
                    let latest = store.get_latest_version(input.substate_id())?;
                    if latest.is_down() {
                        return Err(SubstateStoreError::SubstateNotFound {
                            id: input.with_version(latest.version()).to_owned(),
                        }
                        .into());
                    }
                    debug!(target: LOG_TARGET, "Resolved LOCAL unversioned substate: {input} to version {}",latest.version());
                    local_inputs.insert(input, latest.version());
                },
            }
        }

        let num_local_tombstones = transaction
            .claim_burn_outputs_iter()
            .filter(|tombstone| local_committee_info.includes_substate_id(&SubstateId::from(*tombstone)))
            .count();

        if local_inputs.is_empty() && foreign_inputs.is_empty() && num_local_tombstones == 0 {
            // Mempool/missing transaction validations should have not sent the transaction to consensus
            warn!(target: LOG_TARGET, "⚠️ PREPARE: NEVER HAPPEN transaction {} has no inputs.", transaction.calculate_id());
            return Err(BlockTransactionExecutorError::InvariantError(format!(
                "Transaction {} has no inputs",
                transaction.calculate_id()
            )));
        }

        Ok(ResolvedInputs {
            local_inputs,
            foreign_inputs,
            num_local_tombstones,
        })
    }

    pub fn execute(
        &self,
        execution_epoch: LockedEpoch,
        pledged_transaction: PledgedTransaction,
    ) -> Result<TransactionExecution, BlockTransactionExecutorError> {
        let max_epoch = pledged_transaction.transaction.transaction().max_epoch();
        if let Some(reason) = self.window_abort_reason(execution_epoch.epoch(), max_epoch) {
            warn!(
                target: LOG_TARGET,
                "⏰ Transaction {} is outside its validity window ({reason}): execution epoch {} against max_epoch {}",
                pledged_transaction.transaction.id(),
                execution_epoch,
                max_epoch,
            );
            return Ok(TransactionExecution::abort(
                pledged_transaction.transaction.id(),
                RejectReason::Abort { reason },
            ));
        }

        let resolved_inputs = pledged_transaction
            .local_pledges
            .into_iter()
            .chain(pledged_transaction.foreign_pledges)
            // Exclude any output pledges
            .filter_map(|pledge| pledge.into_input())
            .map(|(id, substate)|
                {
                    let version = id.version();
                    (
                        id.into(),
                        Substate::new(version, substate),
                    )
                })
            .collect();
        self.executor.execute(
            pledged_transaction.transaction.transaction(),
            execution_epoch,
            &resolved_inputs,
        )
    }

    pub fn execute_or_fetch<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        transaction: &TransactionRecord,
        resolved_inputs: &HashMap<SubstateRequirement, Substate>,
        block: &LeafBlock,
        execution_locked_epoch: LockedEpoch,
    ) -> Result<TransactionExecution, BlockTransactionExecutorError> {
        let max_epoch = transaction.transaction().max_epoch();
        if let Some(reason) = self.window_abort_reason(execution_locked_epoch.epoch(), max_epoch) {
            warn!(
                target: LOG_TARGET,
                "⏰ Transaction {} is outside its validity window ({reason}): execution epoch {} against max_epoch {}",
                transaction.id(),
                execution_locked_epoch.epoch(),
                max_epoch,
            );
            return Ok(TransactionExecution::abort(transaction.id(), RejectReason::Abort {
                reason,
            }));
        }

        // Might have been executed already in on propose
        if let Some(execution) = self.fetch_execution(store, transaction.id(), block)? {
            info!(
                target: LOG_TARGET,
                "👨‍🔧 PREPARE: Found existing execution for transaction {}: {}",
                transaction.id(),
                execution
            );
            // Artifacts are stored alongside the execution
            return Ok(execution);
        }
        info!(
            target: LOG_TARGET,
            "👨‍🔧 PREPARE: Executing transaction {}",
            transaction.id(),
        );
        self.executor
            .execute(transaction.transaction(), execution_locked_epoch, resolved_inputs)
    }

    pub fn fetch_execution<TTx: StateStoreReadTransaction>(
        &self,
        store: &PendingSubstateStore<TTx>,
        transaction_id: &TransactionId,
        block: &LeafBlock,
    ) -> Result<Option<TransactionExecution>, BlockTransactionExecutorError> {
        if let Some(execution) =
            BlockTransactionExecution::get_pending_for_block(store.read_transaction(), transaction_id, block)
                .optional()?
        {
            return Ok(Some(execution.into_transaction_execution()));
        }
        Ok(None)
    }

    fn resolve_inputs<'a, TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        transaction: &'a TransactionRecord,
        local_committee_info: &CommitteeInfo,
    ) -> Result<ResolvedTransactionInputs<'a>, BlockTransactionExecutorError> {
        match self.resolve_local_versions(store, local_committee_info, transaction.transaction()) {
            Ok(resolved) => {
                if resolved.local_inputs.is_empty() &&
                    resolved.foreign_inputs.is_empty() &&
                    resolved.num_local_tombstones > 0
                {
                    // EDGE CASE: transaction with only tombstone outputs and no inputs has to be handled differently
                    return Ok(ResolvedTransactionInputs::OnlyTombstones);
                }

                if !resolved.local_inputs.is_empty() && resolved.foreign_inputs.is_empty() {
                    // EDGE CASE: global transaction, but single committee = OnlyLocalInputs behaviour, otherwise
                    // LocalAndForeignInputs behaviour
                    if transaction.transaction().is_global() && local_committee_info.num_committees() > 1 {
                        return Ok(ResolvedTransactionInputs::LocalAndForeignInputs {
                            local_versions: resolved.local_inputs,
                            non_local_inputs: resolved.foreign_inputs,
                        });
                    }

                    return Ok(ResolvedTransactionInputs::OnlyLocalInputs {
                        local_versions: resolved.local_inputs,
                    });
                }

                if resolved.local_inputs.is_empty() && !resolved.foreign_inputs.is_empty() {
                    return Ok(ResolvedTransactionInputs::OnlyForeignInputs {
                        non_local_inputs: resolved.foreign_inputs,
                    });
                }

                Ok(ResolvedTransactionInputs::LocalAndForeignInputs {
                    local_versions: resolved.local_inputs,
                    non_local_inputs: resolved.foreign_inputs,
                })
            },
            Err(err) => {
                warn!(target: LOG_TARGET, "⚠️ PREPARE: failed to resolve local inputs: {err}");
                // We only expect not found or down errors here. If we get any other error, this is fatal.
                if !err.is_not_found_error() {
                    return Err(err);
                }

                let is_local_only = local_committee_info.is_all_local(transaction.transaction.all_inputs_iter()) &&
                    local_committee_info.is_all_local(transaction.transaction.known_outputs_iter());
                Ok(ResolvedTransactionInputs::OneOrMoreLocalInputsNotFound { is_local_only, err })
            },
        }
    }

    fn prepare_local_input_transaction<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        transaction: &TransactionRecord,
        local_versions: IndexMap<SubstateRequirementRef<'_>, u64>,
        block: LeafBlock,
        execution_locked_epoch: LockedEpoch,
    ) -> Result<PreparedTransaction, BlockTransactionExecutorError> {
        let transaction_id = *transaction.id();
        // CASE: All inputs are local and we can execute the transaction.
        //       Outputs may or may not be local

        info!(
            target: LOG_TARGET,
            "👨‍🔧 PREPARE: transaction {} has {} local input(s) and {} tombstone(s)",
            transaction_id,
            local_versions.len(),
            transaction.transaction().claim_burn_iter().count()
        );

        let local_inputs = store.get_many(local_versions.iter().map(|(req, v)| (req.to_owned(), *v)))?;
        let execution = self.execute_or_fetch(store, transaction, &local_inputs, &block, execution_locked_epoch)?;

        let is_outputs_local_only = local_committee_info.is_all_local(execution.resulting_outputs());
        if is_outputs_local_only {
            // CASE: All inputs are local and all outputs are local
            info!(
                target: LOG_TARGET,
                "👨‍🔧 PREPARE: Local-Only Executed transaction {} with {} decision",
                transaction_id,
                execution.decision()
            );

            if let Some(reason) = execution.decision().abort_reason() {
                warn!(target: LOG_TARGET, "⚠️ PREPARE: LocalOnly Aborted: {reason}");
                return Ok(PreparedTransaction::new_local_early_abort(execution));
            }

            let requested_locks = execution.resolved_inputs().iter().chain(execution.resulting_outputs());
            let lock_status = store.try_lock_all(transaction_id, requested_locks, true)?;
            if let Some(err) = lock_status.hard_conflict() {
                warn!(target: LOG_TARGET, "⚠️ PREPARE: Hard conflict when locking inputs: {err}");
                let execution =
                    TransactionExecution::abort(transaction.id(), RejectReason::FailedToLockInputs(err.to_string()));
                return Ok(PreparedTransaction::new_local_accept(execution, lock_status));
            }

            return Ok(PreparedTransaction::new_local_accept(execution, lock_status));
        }

        info!(target: LOG_TARGET, "👨‍🔧 PREPARE: transaction {} has {} local input(s) and foreign outputs (Local decision: {})", transaction_id, local_inputs.len(), execution.decision());
        match execution.decision() {
            Decision::Commit => {
                // CASE: Multishard transaction, all inputs are local, consensus with output shard groups
                // pending
                let requested_locks = execution.resolved_inputs();
                let lock_status = store.try_lock_all(transaction_id, requested_locks, false)?;
                if let Some(err) = lock_status.hard_conflict() {
                    warn!(target: LOG_TARGET, "⚠️ PREPARE: Hard conflict when locking inputs: {err}");
                    let execution = TransactionExecution::abort(
                        transaction.id(),
                        RejectReason::FailedToLockInputs(err.to_string()),
                    );
                    return Ok(PreparedTransaction::new_multishard_executed(execution, lock_status));
                }
                // We're committing, and one or more of the outputs are foreign
                Ok(PreparedTransaction::new_multishard_executed(execution, lock_status))
            },
            Decision::Abort(reason) => {
                // CASE: We're aborting, so this is a local-only transaction since no outputs need to be
                // created
                warn!(target: LOG_TARGET, "⚠️ PREPARE: Aborted: {reason:?}");
                Ok(PreparedTransaction::new_local_early_abort(execution))
            },
        }
    }

    fn prepare_multishard_involved_transaction<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        transaction: &TransactionRecord,
        local_versions: IndexMap<SubstateRequirementRef<'_>, u64>,
        non_local_inputs: IndexSet<SubstateRequirementRef<'_>>,
        block: LeafBlock,
    ) -> Result<PreparedTransaction, BlockTransactionExecutorError> {
        // CASE: Multishard transaction, some inputs are local, some are foreign

        info!(
            target: LOG_TARGET,
            "👨‍🔧 PREPARE: transaction {} has {} local input(s) and {} foreign input(s)",
            transaction.id(),
            local_versions.len(),
            non_local_inputs.len()
        );

        match self.fetch_execution(store, transaction.id(), &block)? {
            Some(exec) => self.prepare_multishard_from_execution(store, local_committee_info, *transaction.id(), exec),
            None => self.prepare_multishard_no_execution(
                store,
                local_committee_info,
                *transaction.id(),
                local_versions,
                non_local_inputs,
            ),
        }
    }

    fn prepare_multishard_from_execution<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        transaction_id: TransactionId,
        execution: TransactionExecution,
    ) -> Result<PreparedTransaction, BlockTransactionExecutorError> {
        if let Some(reason) = execution.decision().abort_reason() {
            warn!(target: LOG_TARGET, "⚠️ PREPARE: Transaction {transaction_id} is ABORTED before prepare: {reason}");
            // No locks
            let lock_status = LockStatus::default();
            return Ok(PreparedTransaction::new_multishard_executed(execution, lock_status));
        }

        let requested_locks = execution
            .resolved_inputs()
            .iter()
            .chain(execution.resulting_outputs())
            .filter(|o| {
                o.substate_id().is_transaction_receipt() || local_committee_info.includes_substate_id(o.substate_id())
            });

        let lock_status = store.try_lock_all(transaction_id, requested_locks, false)?;
        if let Some(err) = lock_status.hard_conflict() {
            // CASE: Multishard transaction, will abort due to hard lock conflict
            warn!(target: LOG_TARGET, "⚠️ PREPARE: Hard conflict when locking inputs: {err}");
            let execution =
                TransactionExecution::abort(&transaction_id, RejectReason::FailedToLockInputs(err.to_string()));
            Ok(PreparedTransaction::new_multishard_executed(execution, lock_status))
        } else {
            // CASE: Multishard transaction, executed, no or "soft" (unversioned) lock conflict
            Ok(PreparedTransaction::new_multishard_executed(execution, lock_status))
        }
    }

    fn prepare_multishard_no_execution<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        transaction_id: TransactionId,
        local_versions: IndexMap<SubstateRequirementRef<'_>, u64>,
        non_local_inputs: IndexSet<SubstateRequirementRef<'_>>,
    ) -> Result<PreparedTransaction, BlockTransactionExecutorError> {
        // TODO: We do not know if the inputs locks required are Read/Write. Either we allow the user to
        //       specify this or we can correct the locks after execution. Currently, this limitation
        //       prevents concurrent multi-shard read locks.
        let requested_locks = local_versions.iter().map(|(req, version)| {
            // TODO: we assume all resources are not being written to. How can we do this in the vast
            // majority of cases but still allow (presumably rare) Access Rule updates?
            if req.substate_id().is_read_only() || req.substate_id().is_resource() {
                SubstateRequirementLockIntent::read(req.to_owned(), *version)
            } else {
                SubstateRequirementLockIntent::write(req.to_owned(), *version)
            }
        });

        let mut evidence = Evidence::from_lock_intents(
            local_committee_info.num_preshards(),
            local_committee_info.num_committees(),
            requested_locks.clone(),
        );
        // Add unpledged foreign input evidence
        for input in non_local_inputs {
            evidence.insert_unpledged_from_substate_id(
                local_committee_info.num_preshards(),
                local_committee_info.num_committees(),
                input.substate_id().clone(),
            );
        }

        // Pledge the local inputs
        let lock_status = store.try_lock_all(transaction_id, requested_locks, false)?;
        info!(
            target: LOG_TARGET,
            "👨‍🔧 PREPARE: Multishard transaction {transaction_id} requires additional input pledges. Partial evidence: {evidence}",
        );
        Ok(PreparedTransaction::new_multishard_evidence(evidence, lock_status))
    }

    fn prepare_multishard_output_only_transaction<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        transaction: &TransactionRecord,
        non_local_inputs: IndexSet<SubstateRequirementRef<'_>>,
        block: LeafBlock,
        change_set: &ProposedBlockChangeSet,
        execution_locked_epoch: LockedEpoch,
    ) -> Result<PreparedTransaction, BlockTransactionExecutorError> {
        let transaction_id = *transaction.id();
        // We're output-only, so only need to gather foreign pledges
        let foreign_pledges = transaction.get_foreign_pledges(store.read_transaction())?;

        info!(
            target: LOG_TARGET,
            "👨‍🔧 PREPARE: Transaction {} is output-only with {} foreign pledge(s)",
            transaction_id,
            foreign_pledges.len()
        );

        let proposed_foreign_pledges = change_set.get_foreign_pledges(&transaction_id);
        let resolved_inputs = foreign_pledges
            .into_iter()
            // NOTE: that duplicate pledges (which should not happen) will be NOT overwritten by the proposed ones due to HashMap from_iter behaviour
            .chain(proposed_foreign_pledges.cloned())
            // Exclude any output pledges
            .filter_map(|pledge| pledge.into_input())
            .map(|(id, substate)|
                {
                    let version = id.version();
                    (
                        // SubstateRequirement
                        id.into(),
                        Substate::new(version, substate),
                    )
                })
            .collect::<HashMap<_, _>>();

        // Check that we have all the required foreign input pledges
        if let Some(req) = non_local_inputs
            .iter()
            .find(|req| !resolved_inputs.contains_key(req.substate_id()))
        {
            error!(target: LOG_TARGET, "⚠️ PREPARE: Missing {req} foreign input pledge for transaction {}", transaction_id);
            // NOTE: crashes consensus. The foreign proposal should have already been verified to include all input
            // pledges.
            return Err(BlockTransactionExecutorError::InvariantError(format!(
                "Missing foreign input pledge for transaction {}: {}",
                transaction_id, req
            )));
        }

        let execution = self.execute_or_fetch(store, transaction, &resolved_inputs, &block, execution_locked_epoch)?;
        info!(
            target: LOG_TARGET,
            "👨‍🔧 PREPARE: Output-Only Executed transaction {} with {} decision",
            transaction_id,
            execution.decision()
        );

        self.prepare_multishard_from_execution(store, local_committee_info, transaction_id, execution)
    }
}

enum ResolvedTransactionInputs<'a> {
    OnlyLocalInputs {
        local_versions: IndexMap<SubstateRequirementRef<'a>, u64>,
    },
    LocalAndForeignInputs {
        local_versions: IndexMap<SubstateRequirementRef<'a>, u64>,
        non_local_inputs: IndexSet<SubstateRequirementRef<'a>>,
    },
    /// Does not involve any local inputs, but involves one or more local tombstone outputs
    OnlyTombstones,
    /// a.k.a "output-only"
    OnlyForeignInputs {
        non_local_inputs: IndexSet<SubstateRequirementRef<'a>>,
    },
    OneOrMoreLocalInputsNotFound {
        is_local_only: bool,
        err: BlockTransactionExecutorError,
    },
}

struct ResolvedInputs<'a> {
    pub local_inputs: IndexMap<SubstateRequirementRef<'a>, u64>,
    pub foreign_inputs: IndexSet<SubstateRequirementRef<'a>>,
    pub num_local_tombstones: usize,
}
