//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
};

use indexmap::IndexMap;
use log::*;
use tari_dan_common_types::{
    committee::CommitteeInfo,
    optional::{IsNotFoundError, Optional},
    Epoch,
    SubstateRequirement,
    ToSubstateAddress,
    VersionedSubstateId,
};
use tari_dan_storage::{
    consensus_models::{
        BlockId,
        BlockTransactionExecution,
        Decision,
        ExecutedTransaction,
        SubstateRequirementLockIntent,
        TransactionExecution,
        TransactionRecord,
    },
    StateStore,
};
use tari_engine_types::{
    commit_result::RejectReason,
    substate::Substate,
    transaction_receipt::TransactionReceiptAddress,
};
use tari_transaction::{Transaction, TransactionId};

use super::{PledgedTransaction, PreparedTransaction};
use crate::{
    hotstuff::substate_store::{LockStatus, PendingSubstateStore},
    tracing::TraceTimer,
    traits::{BlockTransactionExecutor, BlockTransactionExecutorError},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::block_transaction_executor";

#[derive(Debug, Clone)]
pub struct ConsensusTransactionManager<TExecutor, TStateStore> {
    executor: TExecutor,
    _store: PhantomData<TStateStore>,
}

impl<TStateStore: StateStore, TExecutor: BlockTransactionExecutor<TStateStore>>
    ConsensusTransactionManager<TExecutor, TStateStore>
{
    pub fn new(executor: TExecutor) -> Self {
        Self {
            executor,
            _store: PhantomData,
        }
    }

    fn resolve_local_versions(
        &self,
        store: &PendingSubstateStore<TStateStore>,
        local_committee_info: &CommitteeInfo,
        transaction: &Transaction,
    ) -> Result<(IndexMap<SubstateRequirement, u32>, HashSet<SubstateRequirement>), BlockTransactionExecutorError> {
        let mut resolved_substates = IndexMap::with_capacity(transaction.num_unique_inputs());

        let mut non_local_inputs = HashSet::new();
        for input in transaction.all_inputs_iter() {
            if !local_committee_info.includes_substate_id(&input.substate_id) {
                non_local_inputs.insert(input);
                continue;
            }

            match input.version() {
                Some(version) => {
                    let id = VersionedSubstateId::new(input.substate_id, version);
                    store.lock_assert_is_up(&id)?;
                    info!(target: LOG_TARGET, "Resolved LOCAL substate: {id}");
                    resolved_substates.insert(id.into(), version);
                },
                None => {
                    let version = store.get_latest_version(&input.substate_id)?;
                    info!(target: LOG_TARGET, "Resolved LOCAL unversioned substate: {input}");
                    resolved_substates.insert(input, version);
                },
            }
        }
        Ok((resolved_substates, non_local_inputs))
    }

    pub fn execute(
        &self,
        current_epoch: Epoch,
        pledged_transaction: PledgedTransaction,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
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
        let executed = self.executor.execute(
            pledged_transaction.transaction.into_transaction(),
            current_epoch,
            &resolved_inputs,
        )?;

        Ok(executed)
    }

    fn execute_or_fetch(
        &self,
        store: &mut PendingSubstateStore<TStateStore>,
        transaction: Transaction,
        current_epoch: Epoch,
        resolved_inputs: &HashMap<SubstateRequirement, Substate>,
        block_id: &BlockId,
    ) -> Result<TransactionExecution, BlockTransactionExecutorError> {
        info!(
            target: LOG_TARGET,
            "👨‍🔧 PREPARE: Executing transaction {}",
            transaction.id(),
        );
        // Might have been executed already in on propose
        if let Some(execution) =
            BlockTransactionExecution::get_pending_for_block(store.read_transaction(), transaction.id(), block_id)
                .optional()?
        {
            return Ok(execution.into_transaction_execution());
        }

        let executed = self.executor.execute(transaction, current_epoch, resolved_inputs)?;

        Ok(executed.into_execution())
    }

    #[allow(clippy::too_many_lines)]
    pub fn prepare(
        &self,
        store: &mut PendingSubstateStore<TStateStore>,
        local_committee_info: &CommitteeInfo,
        current_epoch: Epoch,
        transaction_id: TransactionId,
        block_id: &BlockId,
    ) -> Result<PreparedTransaction, BlockTransactionExecutorError> {
        let _timer = TraceTimer::info(LOG_TARGET, "prepare");
        let mut transaction = TransactionRecord::get(store.read_transaction(), &transaction_id)?;
        let mut outputs = HashSet::new();
        outputs.insert(VersionedSubstateId::new(
            TransactionReceiptAddress::from(transaction_id).into(),
            0,
        ));

        let (local_versions, non_local_inputs) = match self.resolve_local_versions(
            store,
            local_committee_info,
            transaction.transaction(),
        ) {
            Ok(inputs) => inputs,
            Err(err) => {
                warn!(target: LOG_TARGET, "⚠️ PREPARE: failed to resolve local inputs: {err}");
                // We only expect not found or down errors here. If we get any other error, this is fatal.
                if !err.is_not_found_error() && !err.is_substate_down_error() {
                    return Err(err);
                }
                let is_local_only = local_committee_info.includes_all_substate_addresses(
                    transaction
                        .transaction
                        .all_inputs_iter()
                        .map(|i| i.or_zero_version().to_substate_address()),
                );
                // TODO: consider sending Decision::Abort(AbortReason) in the block.
                // Currently this message will differ depending on which involved shard is asked.
                // e.g. local nodes will say "failed to lock inputs", foreign nodes will say "foreign shard abort"
                transaction.set_abort_reason(RejectReason::OneOrMoreInputsNotFound(err.to_string()));
                if is_local_only {
                    warn!(target: LOG_TARGET, "⚠️ PREPARE: transaction {} only contains local inputs. Will abort locally", transaction_id);
                    return Ok(PreparedTransaction::new_local_early_abort(
                        transaction
                            .into_execution()
                            .expect("invariant: abort reason is set but into_execution is None"),
                    ));
                } else {
                    warn!(target: LOG_TARGET, "⚠️ PREPARE: transaction {} has foreign inputs. Will prepare ABORT", transaction_id);
                    return Ok(PreparedTransaction::new_multishard(
                        transaction.into_execution(),
                        IndexMap::new(),
                        HashSet::new(),
                        outputs,
                        LockStatus::default(),
                    ));
                }
            },
        };

        if local_versions.is_empty() && non_local_inputs.is_empty() {
            // Mempool validations should have not sent the transaction to consensus
            warn!(target: LOG_TARGET, "⚠️ PREPARE: NEVER HAPPEN transaction {transaction_id} has no inputs.");
            return Err(BlockTransactionExecutorError::InvariantError(format!(
                "Transaction {transaction_id} has no inputs"
            )));
        }

        if non_local_inputs.is_empty() {
            // CASE: All inputs are local and we can execute the transaction.
            //       Outputs may or may not be local
            let local_inputs = store.get_many(local_versions.iter().map(|(req, v)| (req.clone(), *v)))?;
            let mut execution = self.execute_or_fetch(
                store,
                transaction.into_transaction(),
                current_epoch,
                &local_inputs,
                block_id,
            )?;

            // local-only transaction can be determined if we've executed the transaction
            let is_local_only = local_committee_info
                .includes_all_substate_addresses(execution.resulting_outputs().iter().map(|o| o.to_substate_address()));
            if is_local_only {
                info!(
                    target: LOG_TARGET,
                    "👨‍🔧 PREPARE: Local-Only Executed transaction {} with {} decision",
                    transaction_id,
                    execution.decision()
                );

                let requested_locks = execution.resolved_inputs().iter().chain(execution.resulting_outputs());
                let lock_status = store.try_lock_all(transaction_id, requested_locks, true)?;
                if let Some(err) = lock_status.hard_conflict() {
                    warn!(target: LOG_TARGET, "⚠️ PREPARE: Hard conflict when locking inputs: {err}");
                    execution.set_abort_reason(RejectReason::FailedToLockInputs(err.to_string()));
                }
                Ok(PreparedTransaction::new_local_accept(execution, lock_status))
            } else {
                info!(target: LOG_TARGET, "👨‍🔧 PREPARE: transaction {} has local inputs and foreign outputs (Local decision: {})", execution.id(), execution.decision());
                match execution.decision() {
                    Decision::Commit => {
                        // CASE: Multishard transaction, all inputs are local, consensus with output shard groups
                        // pending
                        let requested_locks = execution.resolved_inputs();
                        let lock_status = store.try_lock_all(transaction_id, requested_locks, false)?;
                        let all_outputs = execution
                            .resulting_outputs()
                            .iter()
                            .map(|o| o.versioned_substate_id())
                            .cloned()
                            .collect();
                        if let Some(err) = lock_status.hard_conflict() {
                            warn!(target: LOG_TARGET, "⚠️ PREPARE: Hard conflict when locking inputs: {err}");
                            execution.set_abort_reason(RejectReason::FailedToLockInputs(err.to_string()));
                        }
                        // We're committing, and one or more of the outputs are foreign
                        Ok(PreparedTransaction::new_multishard(
                            Some(execution),
                            local_versions,
                            HashSet::new(),
                            all_outputs,
                            lock_status,
                        ))
                    },
                    Decision::Abort => {
                        // CASE: Multishard transaction, but all inputs are local, and we're aborting
                        // All outputs are local, and we're aborting, so this is a local-only transaction since no
                        // outputs need to be created
                        Ok(PreparedTransaction::new_local_early_abort(execution))
                    },
                }
            }
        } else {
            // TODO: We do not know if the inputs locks required are Read/Write. Either we allow the user to
            //       specify this or we can correct the locks after execution. Currently, this limitation
            //       prevents concurrent multi-shard read locks.
            let requested_locks = local_versions.iter().map(|(substate_id, version)| {
                if substate_id.substate_id().is_read_only() {
                    SubstateRequirementLockIntent::read(substate_id.clone(), *version)
                } else {
                    SubstateRequirementLockIntent::write(substate_id.clone(), *version)
                }
            });
            let lock_status = store.try_lock_all(transaction_id, requested_locks, false)?;
            if let Some(err) = lock_status.hard_conflict() {
                warn!(target: LOG_TARGET, "⚠️ PREPARE: Hard conflict when locking inputs: {err}");
                transaction.set_abort_reason(RejectReason::FailedToLockInputs(err.to_string()));
            }
            // CASE: Multishard transaction, not executed
            Ok(PreparedTransaction::new_multishard(
                transaction.into_execution(),
                local_versions,
                non_local_inputs,
                outputs,
                lock_status,
            ))
        }
    }
}
