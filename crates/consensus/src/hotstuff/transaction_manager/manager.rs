//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, hash_map::Entry};

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

use super::{ExecutionMemo, PledgedTransaction, PreparedTransaction, Speculation, UncommittedTemplateStore};
use crate::{
    hotstuff::{
        block_change_set::ProposedBlockChangeSet,
        substate_store::{InputPledge, LockStatus, PendingSubstateStore, SubstateStoreError, SubstateVersionReader},
    },
    tracing::TraceTimer,
    traits::{BlockTransactionExecutor, BlockTransactionExecutorError, ReadableSubstateStore},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::block_transaction_executor";

#[derive(Debug, Clone)]
pub struct ConsensusTransactionManager<TExecutor, TStateStore> {
    executor: TExecutor,
    uncommitted_templates: UncommittedTemplateStore,
    max_transaction_validity_epochs: u64,
    max_transaction_pledge_bytes: usize,
    speculation: Option<Speculation<TStateStore>>,
}

impl<TStateStore: StateStore, TExecutor: BlockTransactionExecutor<TStateStore>>
    ConsensusTransactionManager<TExecutor, TStateStore>
{
    pub fn new(executor: TExecutor, max_transaction_validity_epochs: u64, max_transaction_pledge_bytes: usize) -> Self {
        Self {
            executor,
            uncommitted_templates: UncommittedTemplateStore::default(),
            max_transaction_validity_epochs,
            max_transaction_pledge_bytes,
            speculation: None,
        }
    }

    /// Speculates a block's executions on `execution_threads` threads, reading `state_store` (see [`Self::speculate`]).
    /// With one thread nothing is speculated, and every execution happens as the block is evaluated.
    pub fn with_speculation(mut self, execution_threads: usize, state_store: TStateStore) -> Self {
        self.speculation = Speculation::new(execution_threads, state_store);
        self
    }

    pub(super) fn speculation(&self) -> Option<&Speculation<TStateStore>> {
        self.speculation.as_ref()
    }

    pub(super) fn executor(&self) -> &TExecutor {
        &self.executor
    }

    pub fn uncommitted_templates(&self) -> &UncommittedTemplateStore {
        &self.uncommitted_templates
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

    /// The abort `transaction` earns for executing in a block of `block_epoch` while pinned to
    /// `pinned_epoch`, or `None` if it may execute.
    pub(super) fn execution_abort_reason(
        &self,
        transaction: &Transaction,
        pinned_epoch: Epoch,
        block_epoch: Epoch,
    ) -> Option<AbortReason> {
        self.window_abort_reason(pinned_epoch, transaction.max_epoch())
            .or_else(|| burn_rate_governance_abort_reason(transaction, pinned_epoch, block_epoch))
    }

    /// Prepares the transaction for a block this node evaluates. An execution is taken from `memo` if one was
    /// speculated with the inputs the transaction resolves to here.
    pub fn prepare<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        pool_tx: &TransactionPoolRecord,
        block: LeafBlock,
        change_set: &ProposedBlockChangeSet,
        memo: &mut ExecutionMemo,
    ) -> Result<PreparedTransaction, BlockTransactionExecutorError> {
        self.prepare_inner(store, local_committee_info, pool_tx, block, change_set, memo, false)
    }

    /// Prepares the transaction for a block this node proposes. A proposer defers a transaction with a deferrable lock
    /// conflict, so one whose declared local inputs are locked in a way no execution could lock alongside is
    /// prepared as [PreparedTransaction::InputsLocked] without being executed.
    pub fn prepare_for_proposal<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        pool_tx: &TransactionPoolRecord,
        block: LeafBlock,
        change_set: &ProposedBlockChangeSet,
        memo: &mut ExecutionMemo,
    ) -> Result<PreparedTransaction, BlockTransactionExecutorError> {
        self.prepare_inner(store, local_committee_info, pool_tx, block, change_set, memo, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_inner<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        pool_tx: &TransactionPoolRecord,
        block: LeafBlock,
        change_set: &ProposedBlockChangeSet,
        memo: &mut ExecutionMemo,
        is_proposing: bool,
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
                    memo,
                    is_proposing,
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
                execution_epoch.clone(),
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
                    memo,
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
                    memo,
                    is_proposing,
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

    pub(super) fn resolve_local_versions<'a, S: SubstateVersionReader>(
        &self,
        store: &S,
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

    pub fn execute<TTx: StateStoreReadTransaction>(
        &self,
        store: &PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        execution_epoch: LockedEpoch,
        block_epoch: Epoch,
        pledged_transaction: PledgedTransaction,
    ) -> Result<TransactionExecution, BlockTransactionExecutorError> {
        let transaction = pledged_transaction.transaction.transaction();
        if let Some(reason) = self.execution_abort_reason(transaction, execution_epoch.epoch(), block_epoch) {
            warn!(
                target: LOG_TARGET,
                "⏰ Transaction {} cannot execute in a block of {block_epoch} ({reason}): pinned to {}, max_epoch {}",
                pledged_transaction.transaction.id(),
                execution_epoch,
                transaction.max_epoch(),
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
        let transaction = pledged_transaction.transaction.transaction();
        self.executor.execute(
            transaction,
            execution_epoch,
            input_fanout(transaction, local_committee_info),
            &resolved_inputs,
            &self.uncommitted_templates.templates_for(store)?,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn execute_or_fetch<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        transaction: &TransactionRecord,
        resolved_inputs: &HashMap<SubstateRequirement, Substate>,
        block: &LeafBlock,
        execution_locked_epoch: LockedEpoch,
        memo: &mut ExecutionMemo,
    ) -> Result<TransactionExecution, BlockTransactionExecutorError> {
        if let Some(reason) =
            self.execution_abort_reason(transaction.transaction(), execution_locked_epoch.epoch(), block.epoch)
        {
            warn!(
                target: LOG_TARGET,
                "⏰ Transaction {} cannot execute in a block of {} ({reason}): pinned to {}, max_epoch {}",
                transaction.id(),
                block.epoch,
                execution_locked_epoch.epoch(),
                transaction.transaction().max_epoch(),
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
        if let Some(execution) = memo.take(
            store.parent_block().block_id(),
            transaction.id(),
            resolved_inputs,
            &execution_locked_epoch,
        ) {
            debug!(
                target: LOG_TARGET,
                "👨‍🔧 PREPARE: Using speculated execution for transaction {}: {}",
                transaction.id(),
                execution
            );
            return Ok(execution);
        }
        info!(
            target: LOG_TARGET,
            "👨‍🔧 PREPARE: Executing transaction {}",
            transaction.id(),
        );
        self.executor.execute(
            transaction.transaction(),
            execution_locked_epoch,
            input_fanout(transaction.transaction(), local_committee_info),
            resolved_inputs,
            &self.uncommitted_templates.templates_for(store)?,
        )
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

    #[allow(clippy::too_many_arguments)]
    fn prepare_local_input_transaction<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        transaction: &TransactionRecord,
        local_versions: IndexMap<InputDeclarationRef<'_>, SubstateVersion>,
        block: LeafBlock,
        execution_locked_epoch: LockedEpoch,
        memo: &mut ExecutionMemo,
        is_proposing: bool,
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

        // A transaction outside its validity window aborts without executing, so it is not worth deferring.
        let max_epoch = transaction.transaction().max_epoch();
        if is_proposing &&
            self.window_abort_reason(execution_locked_epoch.epoch(), max_epoch)
                .is_none()
        {
            let lock_status = store.check_input_locks(
                &transaction_id,
                local_versions
                    .iter()
                    .map(|(decl, version)| declared_lock_intent(decl, *version)),
            )?;
            if lock_status.is_deferrable_conflict() {
                info!(
                    target: LOG_TARGET,
                    "👨‍🔧 PREPARE: transaction {} has local input(s) locked by another transaction. Not executing.",
                    transaction_id,
                );
                return Ok(PreparedTransaction::InputsLocked { lock_status });
            }
        }

        let local_inputs = store.get_many(
            local_versions
                .iter()
                .map(|(req, v)| (req.to_substate_requirement_ref().to_owned(), *v)),
        )?;
        let execution = self.execute_or_fetch(
            store,
            local_committee_info,
            transaction,
            &local_inputs,
            &block,
            execution_locked_epoch,
            memo,
        )?;

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
        execution_locked_epoch: LockedEpoch,
    ) -> Result<PreparedTransaction, BlockTransactionExecutorError> {
        // CASE: Multishard transaction, some inputs are local, some are foreign

        info!(
            target: LOG_TARGET,
            "👨‍🔧 PREPARE: transaction {} has {} local input(s) and {} foreign input(s)",
            transaction.id(),
            local_versions.len(),
            non_local_inputs.len()
        );

        // Without every input this shard group cannot execute, so a window that is too long is rejected here, before
        // any input is locked. The LocalPrepared timeout is measured from max_epoch, so it only bounds how long locks
        // are held if max_epoch is within the window. The other shard groups receive and adopt the abort.
        // An expired max_epoch is left to execution: this group's pin may still be lowered by another group's, and
        // execution judges expiry at the final pin. Its locks are bounded by the LocalPrepared timeout meanwhile.
        // A window too long at this pin is too long at any lower pin, so that verdict cannot change.
        let max_epoch = transaction.transaction().max_epoch();
        if let Some(reason @ AbortReason::ValidityWindowTooLong) =
            self.window_abort_reason(execution_locked_epoch.epoch(), max_epoch)
        {
            warn!(
                target: LOG_TARGET,
                "⏰ PREPARE: Transaction {} declares a validity window that is too long: pinned to {}, max_epoch {max_epoch}",
                transaction.id(),
                execution_locked_epoch,
            );
            let execution = TransactionExecution::abort(transaction.id(), RejectReason::Abort { reason });
            return Ok(PreparedTransaction::new_multishard_executed(
                execution,
                LockStatus::default(),
            ));
        }

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

    #[allow(clippy::too_many_arguments)]
    fn prepare_multishard_output_only_transaction<TTx: StateStoreReadTransaction>(
        &self,
        store: &mut PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        transaction: &TransactionRecord,
        non_local_inputs: IndexSet<InputDeclarationRef<'_>>,
        block: LeafBlock,
        change_set: &ProposedBlockChangeSet,
        execution_locked_epoch: LockedEpoch,
        memo: &mut ExecutionMemo,
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

        let execution = self.execute_or_fetch(
            store,
            local_committee_info,
            transaction,
            &resolved_inputs,
            &block,
            execution_locked_epoch,
            memo,
        )?;
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

pub(super) struct ResolvedInputs<'a> {
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
pub(super) fn declared_lock_intent(
    decl: &InputDeclarationRef<'_>,
    version: SubstateVersion,
) -> SubstateRequirementLockIntent {
    let requirement = decl.to_substate_requirement_ref().to_owned();
    if decl.is_write() && !decl.substate_id().is_read_only() {
        SubstateRequirementLockIntent::write(requirement, version)
    } else {
        SubstateRequirementLockIntent::read(requirement, version)
    }
}

/// [`Transaction::input_fanout`] under the shard layout of the block executing the transaction.
pub(super) fn input_fanout(transaction: &Transaction, local_committee_info: &CommitteeInfo) -> u32 {
    transaction.input_fanout(
        local_committee_info.num_preshards(),
        local_committee_info.num_committees(),
    )
}

/// The abort a transaction declaring the burn rate governance component earns for executing in a
/// later epoch than the one it was pinned to, or `None` if it may execute.
///
/// The template holds an activation to a lead time measured from `Consensus::current_epoch()`, which
/// is the pinned epoch. Consensus resolves an epoch's rate from committed state on the premise that
/// the lead time runs from the epoch the call actually executes in, so the two epochs must be the
/// same. Every shard group holds the component and executes the call, and an abort from any one of
/// them aborts it everywhere.
fn burn_rate_governance_abort_reason(
    transaction: &Transaction,
    pinned_epoch: Epoch,
    block_epoch: Epoch,
) -> Option<AbortReason> {
    if block_epoch == pinned_epoch {
        return None;
    }
    transaction
        .all_inputs_substate_ids_iter()
        .any(|id| id.is_burn_rate_governance())
        .then_some(AbortReason::GovernanceCallAfterPinnedEpoch)
}

#[cfg(test)]
mod tests {
    use tari_common_types::types::PrivateKey;
    use tari_consensus_types::BlockId;
    use tari_engine_types::{
        non_fungible::NonFungibleContainer,
        substate::{SubstateId, SubstateValue},
    };
    use tari_ootle_common_types::{
        InputDeclarationRef,
        NumPreshards,
        ShardGroup,
        SubstateLockType,
        SubstateRequirement,
        SubstateVersion,
        VersionedSubstateId,
        VotePower,
        shard::Shard,
    };
    use tari_ootle_storage::consensus_models::{Block, LockedEpoch, SubstateChange, SubstatePledge, TransactionRecord};
    use tari_ootle_transaction::{Network, Transaction, TransactionId};
    use tari_state_store_rocksdb::RocksDbStateStore;
    use tari_template_lib_types::{
        ComponentAddress,
        ObjectKey,
        constants::{
            BURN_RATE_GOVERNANCE_COMPONENT_ADDRESS,
            PUBLIC_IDENTITY_RESOURCE_ADDRESS,
            STEALTH_TARI_RESOURCE_ADDRESS,
        },
    };

    use super::*;
    use crate::{
        hotstuff::transaction_manager::EvidenceOrExecution,
        traits::{UncommittedChainTemplates, WriteableSubstateStore},
    };

    /// Stands in for the engine: any transaction that reaches it fails, so a test can tell an
    /// execution from an abort consensus decided before executing.
    struct RefusingExecutor;

    impl BlockTransactionExecutor<RocksDbStateStore<String>> for RefusingExecutor {
        fn execute(
            &self,
            _transaction: &Transaction,
            _locked_epoch: LockedEpoch,
            _input_fanout: u32,
            _resolved_inputs: &HashMap<SubstateRequirement, Substate>,
            _templates: &UncommittedChainTemplates,
        ) -> Result<TransactionExecution, BlockTransactionExecutorError> {
            Err(BlockTransactionExecutorError::ExecutionThreadFailure(
                "reached the engine".to_string(),
            ))
        }
    }

    fn manager() -> ConsensusTransactionManager<RefusingExecutor, RocksDbStateStore<String>> {
        ConsensusTransactionManager::new(RefusingExecutor, 2160, usize::MAX)
    }

    fn calling(component: ComponentAddress) -> PledgedTransaction {
        let transaction = Transaction::builder_localnet(Epoch(100))
            .add_input(SubstateRequirement::unversioned(component))
            .build_and_seal(&PrivateKey::from(1u64));
        PledgedTransaction {
            transaction: TransactionRecord::new(transaction),
            foreign_pledges: Vec::new(),
            local_pledges: Vec::new(),
        }
    }

    fn pinned_to(epoch: u64) -> LockedEpoch {
        LockedEpoch::new(Epoch(epoch), Default::default(), Default::default())
    }

    /// The committee of a network with one shard group, in `epoch`.
    fn single_committee(epoch: Epoch) -> CommitteeInfo {
        CommitteeInfo::new(
            NumPreshards::P256,
            1,
            1,
            ShardGroup::all_shards(NumPreshards::P256),
            epoch,
            VotePower::of(1),
        )
    }

    /// Executes `pledged` in a block of `block_epoch` that builds on an empty chain.
    fn execute_in_block_of(
        execution_epoch: LockedEpoch,
        block_epoch: Epoch,
        pledged: PledgedTransaction,
    ) -> Result<TransactionExecution, BlockTransactionExecutorError> {
        let dir = tempfile::tempdir().unwrap();
        let store = RocksDbStateStore::<String>::open(dir.path().join("db"), Default::default()).unwrap();
        let block = LeafBlock {
            block_id: BlockId::zero(),
            height: Default::default(),
            epoch: block_epoch,
            shard_group: ShardGroup::all_shards(NumPreshards::P256),
        };
        store.with_read_tx(|tx| {
            let substate_store = PendingSubstateStore::init(tx, block, NumPreshards::P256)?;
            manager().execute(
                &substate_store,
                &single_committee(block_epoch),
                execution_epoch,
                block_epoch,
                pledged,
            )
        })
    }

    #[test]
    fn a_governance_call_executing_after_its_pinned_epoch_aborts() {
        let execution = execute_in_block_of(pinned_to(5), Epoch(6), calling(BURN_RATE_GOVERNANCE_COMPONENT_ADDRESS))
            .expect("aborts before reaching the engine");
        assert_eq!(
            execution.decision().abort_reason(),
            Some(AbortReason::GovernanceCallAfterPinnedEpoch)
        );
    }

    #[test]
    fn a_governance_call_prepared_after_its_pinned_epoch_aborts() {
        let dir = tempfile::tempdir().unwrap();
        let store = RocksDbStateStore::<String>::open(dir.path().join("db"), Default::default()).unwrap();
        let block = LeafBlock {
            block_id: BlockId::zero(),
            height: Default::default(),
            epoch: Epoch(6),
            shard_group: ShardGroup::all_shards(NumPreshards::P256),
        };
        let pledged = calling(BURN_RATE_GOVERNANCE_COMPONENT_ADDRESS);

        let execution = store
            .with_read_tx(|tx| {
                let mut substate_store = PendingSubstateStore::init(tx, block, NumPreshards::P256)?;
                manager().execute_or_fetch(
                    &mut substate_store,
                    &single_committee(Epoch(6)),
                    &pledged.transaction,
                    &HashMap::new(),
                    &block,
                    pinned_to(5),
                    &mut ExecutionMemo::new(block.block_id),
                )
            })
            .expect("aborts before reaching the engine");
        assert_eq!(
            execution.decision().abort_reason(),
            Some(AbortReason::GovernanceCallAfterPinnedEpoch)
        );
    }

    #[test]
    fn a_governance_call_executing_in_its_pinned_epoch_executes() {
        let result = execute_in_block_of(pinned_to(5), Epoch(5), calling(BURN_RATE_GOVERNANCE_COMPONENT_ADDRESS));
        assert!(matches!(
            result,
            Err(BlockTransactionExecutorError::ExecutionThreadFailure(_))
        ));
    }

    #[test]
    fn an_ordinary_call_executing_after_its_pinned_epoch_executes() {
        let component = ComponentAddress::new(ObjectKey::from_array([1; 32]));
        let result = execute_in_block_of(pinned_to(5), Epoch(6), calling(component));
        assert!(matches!(
            result,
            Err(BlockTransactionExecutorError::ExecutionThreadFailure(_))
        ));
    }

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

    fn shard_of(id: &SubstateId) -> Shard {
        VersionedSubstateIdRef::new(id, SubstateVersion::ZERO).to_shard(NumPreshards::P256)
    }

    /// Prepares, pinned to epoch 1, a transaction with one input in this shard group and one in
    /// another. Returns the prepared decision and whether the local input was locked.
    fn prepare_cross_shard(max_epoch: Epoch) -> (Decision, bool) {
        let local = component_id(1);
        let local_shard = shard_of(&local);
        let foreign = (2..=u8::MAX)
            .map(component_id)
            .find(|id| shard_of(id) != local_shard)
            .unwrap();
        let committee = CommitteeInfo::new(
            NumPreshards::P256,
            1,
            2,
            ShardGroup::new(local_shard, local_shard),
            Epoch(1),
            VotePower::of(1),
        );
        let transaction = TransactionRecord::new(
            Transaction::builder_localnet(max_epoch)
                .add_input(SubstateRequirement::unversioned(local.clone()))
                .add_input(SubstateRequirement::unversioned(foreign))
                .build_and_seal(&PrivateKey::from(1u64)),
        );

        let dir = tempfile::tempdir().unwrap();
        let db = RocksDbStateStore::<String>::open(dir.path().join("db"), Default::default()).unwrap();
        let zero = Block::zero_block(Network::LocalNet, NumPreshards::P256);
        db.with_write_tx(|tx| zero.insert(tx)).unwrap();
        let block = zero.as_leaf();
        db.with_read_tx(|tx| {
            let mut store = PendingSubstateStore::init(tx, block, NumPreshards::P256)?;
            // The builder also declares the fee resource, which may share the local shard
            for input in transaction.transaction().all_inputs_iter() {
                if committee.includes_substate_id(input.substate_id()) {
                    store.put(SubstateChange::Up {
                        id: input.substate_id().clone(),
                        shard: local_shard,
                        substate: Box::new(Substate::new(SubstateVersion::ZERO, NonFungibleContainer::no_data())),
                    })?;
                }
            }
            let manager = manager();
            let ResolvedTransactionInputs::LocalAndForeignInputs {
                local_versions,
                non_local_inputs,
            } = manager.resolve_inputs(&mut store, &transaction, &committee)?
            else {
                panic!("expected local and foreign inputs");
            };
            let PreparedTransaction::MultiShard(prepared) = manager.prepare_multishard_involved_transaction(
                &mut store,
                &committee,
                &transaction,
                local_versions,
                non_local_inputs,
                block,
                pinned_to(1),
            )?
            else {
                panic!("expected a multishard preparation");
            };
            let decision = match prepared.into_evidence_or_execution() {
                EvidenceOrExecution::Execution { execution } => execution.decision(),
                EvidenceOrExecution::Evidence { .. } => Decision::Commit,
            };
            Ok::<_, BlockTransactionExecutorError>((decision, store.new_locks().contains_key(&local)))
        })
        .unwrap()
    }

    #[test]
    fn a_cross_shard_transaction_outside_its_window_aborts_without_locking() {
        let (decision, is_locked) = prepare_cross_shard(Epoch(u64::MAX));
        assert_eq!(decision.abort_reason(), Some(AbortReason::ValidityWindowTooLong));
        assert!(!is_locked);
    }

    #[test]
    fn a_cross_shard_transaction_expired_at_this_pin_still_locks_its_local_input() {
        let (decision, is_locked) = prepare_cross_shard(Epoch(0));
        assert!(decision.is_commit());
        assert!(is_locked);
    }

    #[test]
    fn a_cross_shard_transaction_inside_its_window_locks_its_local_input() {
        let (decision, is_locked) = prepare_cross_shard(Epoch(2));
        assert!(decision.is_commit());
        assert!(is_locked);
    }
}
