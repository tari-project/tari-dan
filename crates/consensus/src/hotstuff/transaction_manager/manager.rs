//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, hash_map::Entry},
    marker::PhantomData,
};

use indexmap::{IndexMap, IndexSet};
use log::*;
use tari_consensus_types::{Decision, LeafBlock};
use tari_engine_types::{
    commit_result::{AbortReason, RejectReason},
    substate::{Substate, SubstateId},
};
use tari_ootle_common_types::{
    Epoch,
    InputDeclarationRef,
    LockIntent,
    SubstateRequirement,
    SubstateVersion,
    VersionedSubstateIdRef,
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
        SubstatePledge,
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
        substate_store::{InputPledge, LockStatus, PendingSubstateStore, SubstateStoreError},
    },
    tracing::TraceTimer,
    traits::{BlockTransactionExecutor, BlockTransactionExecutorError, ReadableSubstateStore},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::block_transaction_executor";

#[derive(Debug, Clone)]
pub struct ConsensusTransactionManager<TExecutor, TStateStore> {
    executor: TExecutor,
    max_transaction_validity_epochs: u64,
    max_transaction_pledge_bytes: usize,
    _store: PhantomData<TStateStore>,
}

impl<TStateStore: StateStore, TExecutor: BlockTransactionExecutor<TStateStore>>
    ConsensusTransactionManager<TExecutor, TStateStore>
{
    pub fn new(executor: TExecutor, max_transaction_validity_epochs: u64, max_transaction_pledge_bytes: usize) -> Self {
        Self {
            executor,
            max_transaction_validity_epochs,
            max_transaction_pledge_bytes,
            _store: PhantomData,
        }
    }

    /// Adds the transaction's local inputs to what the block pledges to foreign shard groups, before they are
    /// locked. Returns what the transaction is prepared as instead if they cannot be pledged: an abort if they are
    /// more than any block may pledge, or a deferral if they only do not fit in this block. Only the shard group
    /// holding the inputs measures them, so it aborts unilaterally and the other shard groups adopt the abort from
    /// its foreign proposal.
    fn pledge_local_inputs<'i, TTx: StateStoreReadTransaction, I>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        transaction_id: &TransactionId,
        inputs: I,
    ) -> Result<Option<PreparedTransaction>, BlockTransactionExecutorError>
    where
        I: IntoIterator<Item = VersionedSubstateIdRef<'i>>,
    {
        match store.pledge_inputs(inputs, self.max_transaction_pledge_bytes)? {
            InputPledge::Pledged => Ok(None),
            InputPledge::TransactionTooLarge { bytes } => {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ PREPARE: Transaction {transaction_id} pledges {bytes} bytes of local inputs, more than the {} a \
                     block may pledge. Aborting.",
                    self.max_transaction_pledge_bytes,
                );
                let execution = TransactionExecution::abort(transaction_id, RejectReason::Abort {
                    reason: AbortReason::InputPledgeTooLarge,
                });
                Ok(Some(PreparedTransaction::new_multishard_executed(
                    execution,
                    LockStatus::new(),
                )))
            },
            InputPledge::BlockBudgetReached { total_bytes, max_bytes } => {
                Ok(Some(PreparedTransaction::PledgeBudgetReached {
                    total_bytes,
                    max_bytes,
                }))
            },
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
        local_versions: IndexMap<InputDeclarationRef<'_>, SubstateVersion>,
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

        let local_inputs = store.get_many(
            local_versions
                .iter()
                .map(|(req, v)| (req.to_substate_requirement_ref().to_owned(), *v)),
        )?;
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
                if let Some(prepared) = self.pledge_local_inputs(
                    store,
                    &transaction_id,
                    requested_locks
                        .iter()
                        .filter(|lock| local_committee_info.includes_substate_id(lock.substate_id()))
                        .map(|lock| lock.to_versioned_substate_id_ref()),
                )? {
                    return Ok(prepared);
                }
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
        local_versions: IndexMap<InputDeclarationRef<'_>, SubstateVersion>,
        non_local_inputs: IndexSet<InputDeclarationRef<'_>>,
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

        if let Some(prepared) = self.pledge_local_inputs(
            store,
            &transaction_id,
            execution
                .resolved_inputs()
                .iter()
                .filter(|lock| local_committee_info.includes_substate_id(lock.substate_id()))
                .map(|lock| lock.to_versioned_substate_id_ref()),
        )? {
            return Ok(prepared);
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
        local_versions: IndexMap<InputDeclarationRef<'_>, SubstateVersion>,
        non_local_inputs: IndexSet<InputDeclarationRef<'_>>,
    ) -> Result<PreparedTransaction, BlockTransactionExecutorError> {
        // This shard group does not hold every input, so it cannot execute to discover how each one is
        // used and must lock on what the transaction declared.
        let requested_locks = local_versions
            .iter()
            .map(|(decl, version)| {
                let substate = store.get(VersionedSubstateIdRef::new(decl.substate_id(), *version))?;
                Ok(declared_lock_intent(decl, *version).with_pledged_value(substate.substate_value()))
            })
            .collect::<Result<Vec<_>, SubstateStoreError>>()?;

        if let Some(prepared) = self.pledge_local_inputs(
            store,
            &transaction_id,
            requested_locks.iter().map(|lock| lock.to_versioned_substate_id_ref()),
        )? {
            return Ok(prepared);
        }

        let mut evidence = Evidence::from_lock_intents(
            local_committee_info.num_preshards(),
            local_committee_info.num_committees(),
            &requested_locks,
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
        let lock_status = store.try_lock_all(transaction_id, &requested_locks, false)?;
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
        non_local_inputs: IndexSet<InputDeclarationRef<'_>>,
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
        let resolved_inputs = resolve_foreign_input_pledges(
            &transaction_id,
            foreign_pledges.into_iter().chain(proposed_foreign_pledges.cloned()),
        )?;

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
        local_versions: IndexMap<InputDeclarationRef<'a>, SubstateVersion>,
    },
    LocalAndForeignInputs {
        local_versions: IndexMap<InputDeclarationRef<'a>, SubstateVersion>,
        non_local_inputs: IndexSet<InputDeclarationRef<'a>>,
    },
    /// Does not involve any local inputs, but involves one or more local tombstone outputs
    OnlyTombstones,
    /// a.k.a "output-only"
    OnlyForeignInputs {
        non_local_inputs: IndexSet<InputDeclarationRef<'a>>,
    },
    OneOrMoreLocalInputsNotFound {
        is_local_only: bool,
        err: BlockTransactionExecutorError,
    },
}

struct ResolvedInputs<'a> {
    pub local_inputs: IndexMap<InputDeclarationRef<'a>, SubstateVersion>,
    pub foreign_inputs: IndexSet<InputDeclarationRef<'a>>,
    pub num_local_tombstones: usize,
}

/// Collects the input pledges among `pledges` into the inputs a transaction executes against, excluding output
/// pledges. Every pledge for a substate must be at the same version. A pledge's value is authenticated against its
/// evidence's value hash when the foreign proposal is accepted (`BlockPledge::get_committed_pledge`), so only the
/// version is compared here.
fn resolve_foreign_input_pledges(
    transaction_id: &TransactionId,
    pledges: impl IntoIterator<Item = SubstatePledge>,
) -> Result<HashMap<SubstateRequirement, Substate>, BlockTransactionExecutorError> {
    let mut resolved_inputs = HashMap::new();
    // Exclude any output pledges
    for (id, substate) in pledges.into_iter().filter_map(|pledge| pledge.into_input()) {
        let version = id.version();
        match resolved_inputs.entry(SubstateRequirement::from(id)) {
            Entry::Occupied(entry) => {
                let existing: &Substate = entry.get();
                if existing.version() != version {
                    return Err(BlockTransactionExecutorError::InvariantError(format!(
                        "Foreign pledges for transaction {transaction_id} pledge substate {} at both v{} and \
                         v{version}",
                        entry.key().substate_id(),
                        existing.version(),
                    )));
                }
            },
            Entry::Vacant(entry) => {
                entry.insert(Substate::new(version, substate));
            },
        }
    }
    Ok(resolved_inputs)
}

/// The lock a shard group takes on an input it holds but cannot execute against, from the transaction's
/// declaration alone. Writing to a read-declared input aborts in the engine, so a read lock cannot be escaped by
/// the transaction that asked for it. A substate that no transaction can write takes a read lock whatever was
/// declared, so that the default write declaration does not serialise every transaction that uses it.
fn declared_lock_intent(decl: &InputDeclarationRef<'_>, version: SubstateVersion) -> SubstateRequirementLockIntent {
    let requirement = decl.to_substate_requirement_ref().to_owned();
    if decl.is_write() && !decl.substate_id().is_read_only() {
        SubstateRequirementLockIntent::write(requirement, version)
    } else {
        SubstateRequirementLockIntent::read(requirement, version)
    }
}

#[cfg(test)]
mod tests {
    use tari_engine_types::{
        non_fungible::NonFungibleContainer,
        substate::{SubstateId, SubstateValue},
    };
    use tari_ootle_common_types::{
        InputDeclarationRef,
        SubstateLockType,
        SubstateRequirement,
        SubstateVersion,
        VersionedSubstateId,
    };
    use tari_ootle_storage::consensus_models::SubstatePledge;
    use tari_ootle_transaction::TransactionId;
    use tari_template_lib_types::{
        ComponentAddress,
        ObjectKey,
        constants::{PUBLIC_IDENTITY_RESOURCE_ADDRESS, STEALTH_TARI_RESOURCE_ADDRESS},
    };

    use super::{declared_lock_intent, resolve_foreign_input_pledges};
    use crate::traits::BlockTransactionExecutorError;

    fn lock_type(decl: InputDeclarationRef<'_>) -> SubstateLockType {
        declared_lock_intent(&decl, SubstateVersion::ZERO).lock_type()
    }

    #[test]
    fn a_write_declaration_on_a_writable_substate_takes_a_write_lock() {
        let id = SubstateId::Component(ComponentAddress::new(ObjectKey::from_array([1; 32])));
        assert_eq!(lock_type(InputDeclarationRef::write(&id)), SubstateLockType::Write);
        assert_eq!(lock_type(InputDeclarationRef::read(&id)), SubstateLockType::Read);
    }

    #[test]
    fn a_write_declaration_on_a_read_only_substate_takes_a_read_lock() {
        for id in [
            SubstateId::Resource(STEALTH_TARI_RESOURCE_ADDRESS),
            SubstateId::Resource(PUBLIC_IDENTITY_RESOURCE_ADDRESS),
        ] {
            assert!(id.is_read_only());
            assert_eq!(lock_type(InputDeclarationRef::write(&id)), SubstateLockType::Read);
        }
    }

    fn component_id(n: u8) -> SubstateId {
        SubstateId::Component(ComponentAddress::new(ObjectKey::from_array([n; 32])))
    }

    fn input_pledge(id: &SubstateId, version: u64) -> SubstatePledge {
        SubstatePledge::Input {
            substate_id: VersionedSubstateId::new(id.clone(), SubstateVersion::new(version)),
            is_write: true,
            substate: Box::new(SubstateValue::from(NonFungibleContainer::no_data())),
        }
    }

    #[test]
    fn duplicate_input_pledges_at_the_same_version_resolve_to_one_input() {
        let id = component_id(1);
        let resolved =
            resolve_foreign_input_pledges(&TransactionId::default(), [input_pledge(&id, 3), input_pledge(&id, 3)])
                .unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[&SubstateRequirement::unversioned(id)].version(),
            SubstateVersion::new(3)
        );
    }

    #[test]
    fn input_pledges_for_one_substate_at_different_versions_are_an_invariant_error() {
        let id = component_id(1);
        let err =
            resolve_foreign_input_pledges(&TransactionId::default(), [input_pledge(&id, 3), input_pledge(&id, 4)])
                .unwrap_err();
        assert!(matches!(err, BlockTransactionExecutorError::InvariantError(_)));
    }

    #[test]
    fn output_pledges_are_not_resolved_as_inputs() {
        let input = component_id(1);
        let output = component_id(2);
        let resolved = resolve_foreign_input_pledges(&TransactionId::default(), [
            input_pledge(&input, 0),
            SubstatePledge::Output {
                substate_id: VersionedSubstateId::new(output.clone(), SubstateVersion::ZERO),
            },
        ])
        .unwrap();
        assert_eq!(resolved.len(), 1);
        assert!(resolved.contains_key(&input));
        assert!(!resolved.contains_key(&output));
    }
}
