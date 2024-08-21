//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, marker::PhantomData};

use indexmap::IndexMap;
use log::*;
use tari_dan_common_types::{
    committee::CommitteeInfo,
    optional::{IsNotFoundError, Optional},
    Epoch,
};
use tari_dan_storage::{
    consensus_models::{
        Decision,
        ExecutedTransaction,
        SubstateLockType,
        TransactionRecord,
        VersionedSubstateIdLockIntent,
    },
    StateStore,
};
use tari_engine_types::{
    commit_result::RejectReason,
    substate::{Substate, SubstateId},
};
use tari_transaction::{SubstateRequirement, Transaction, TransactionId, VersionedSubstateId};

use super::{LocalPreparedTransaction, PledgedTransaction, PreparedTransaction};
use crate::{
    hotstuff::substate_store::PendingSubstateStore,
    traits::{BlockTransactionExecutor, BlockTransactionExecutorError, ReadableSubstateStore},
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

    fn resolve_local_substates(
        &self,
        store: &PendingSubstateStore<TStateStore>,
        local_committee_info: &CommitteeInfo,
        transaction: &Transaction,
    ) -> Result<(IndexMap<SubstateId, Substate>, HashSet<SubstateRequirement>), BlockTransactionExecutorError> {
        let mut resolved_substates = IndexMap::with_capacity(transaction.num_unique_inputs());

        let mut non_local_inputs = HashSet::new();
        for input in transaction.all_inputs_iter() {
            match input.version() {
                Some(version) => {
                    if !local_committee_info.includes_substate_address(
                        &input.to_substate_address().expect("succeeds because version is Some"),
                    ) {
                        non_local_inputs.insert(input);
                        continue;
                    }

                    let id = VersionedSubstateId::new(input.substate_id, version);
                    let substate = store.get(&id)?;
                    info!(target: LOG_TARGET, "Resolved LOCAL substate: {id}");
                    resolved_substates.insert(id.substate_id, substate);
                },
                None => {
                    let substate = match store.get_latest(&input.substate_id).optional()? {
                        Some(substate) => substate,
                        None => {
                            non_local_inputs.insert(input);
                            continue;
                        },
                    };
                    info!(target: LOG_TARGET, "Resolved LOCAL unversioned substate: {input}");
                    resolved_substates.insert(input.substate_id, substate);
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
                        id.substate_id,
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

    #[allow(clippy::too_many_lines)]
    pub fn prepare(
        &self,
        store: &mut PendingSubstateStore<TStateStore>,
        local_committee_info: &CommitteeInfo,
        current_epoch: Epoch,
        transaction_id: TransactionId,
    ) -> Result<PreparedTransaction, BlockTransactionExecutorError> {
        let mut transaction = TransactionRecord::get(store.read_transaction(), &transaction_id)?;
        // Get the latest input substates
        let (local_inputs, non_local_inputs) = match self.resolve_local_substates(
            store,
            local_committee_info,
            transaction.transaction(),
        ) {
            Ok(inputs) => inputs,
            Err(err) => {
                warn!(target: LOG_TARGET, "⚠️ PREPARE: failed to resolve local inputs: {err}");
                // We only expect not found errors here. If we get any other error, this is fatal.
                if !err.is_not_found_error() {
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
                    return Ok(PreparedTransaction::new_local_early_abort(transaction));
                } else {
                    warn!(target: LOG_TARGET, "⚠️ PREPARE: transaction {} has foreign inputs. Will prepare ABORT", transaction_id);
                    return Ok(PreparedTransaction::new_multishard(
                        transaction,
                        IndexMap::new(),
                        HashSet::new(),
                        HashSet::new(),
                    ));
                }
            },
        };

        if local_inputs.is_empty() && non_local_inputs.is_empty() {
            // CASE: Invalid transaction, no inputs
            warn!(target: LOG_TARGET, "⚠️ PREPARE: transaction {transaction_id} has no inputs. Aborting...");
            transaction.set_abort_reason(RejectReason::NoInputs);
            return Ok(PreparedTransaction::new_local_early_abort(transaction));
        }

        let mut prepared = if non_local_inputs.is_empty() {
            // CASE: All inputs are local and we can execute the transaction.
            //       Outputs may or may not be local
            let executed = self
                .executor
                .execute(transaction.into_transaction(), current_epoch, &local_inputs)?;

            // local-only transaction can be determined if we've executed the transaction
            let is_local_only = local_committee_info
                .includes_all_substate_addresses(executed.resulting_outputs().iter().map(|o| o.to_substate_address()));
            if is_local_only {
                info!(
                    target: LOG_TARGET,
                    "👨‍🔧 PREPARE: Local-Only Executed transaction {} with {} decision",
                    executed.id(),
                    executed.decision()
                );
                PreparedTransaction::new_local_accept(executed)
            } else {
                info!(target: LOG_TARGET, "👨‍🔧 PREPARE: transaction {} has local inputs and foreign outputs (Local decision: {})", executed.id(), executed.decision());
                match executed.decision() {
                    Decision::Commit => {
                        // CASE: Multishard transaction, all inputs are local, consensus with output shard groups
                        // pending
                        let all_outputs = executed
                            .resulting_outputs()
                            .iter()
                            .map(|o| o.versioned_substate_id())
                            .cloned()
                            .collect();
                        // We're committing, and one or more of the outputs are foreign
                        PreparedTransaction::new_multishard(executed.into(), local_inputs, HashSet::new(), all_outputs)
                    },
                    Decision::Abort => {
                        // CASE: Multishard transaction, but all inputs are local, and we're aborting
                        // All outputs are local, and we're aborting, so this is a local-only transaction since no
                        // outputs need to be created
                        PreparedTransaction::new_local_early_abort(executed.into())
                    },
                }
            }
        } else {
            // CASE: Multishard transaction, not executed
            PreparedTransaction::new_multishard(transaction, local_inputs, non_local_inputs, HashSet::new())
        };

        let lock_result = match &prepared {
            PreparedTransaction::LocalOnly(LocalPreparedTransaction::Accept(executed)) => {
                let requested_locks = executed.resolved_inputs().iter().chain(executed.resulting_outputs());
                store.try_lock_all(transaction_id, requested_locks, true)
            },
            PreparedTransaction::LocalOnly(LocalPreparedTransaction::EarlyAbort { .. }) => {
                // ABORT - No locks
                Ok(())
            },
            PreparedTransaction::MultiShard(multishard) => {
                if multishard.transaction().current_decision().is_commit() {
                    // TODO: We do not know if the inputs locks required are Read/Write. Either we allow the user to
                    //       specify this or we can correct the locks after execution. Currently, this limitation
                    //       prevents concurrent multi-shard read locks.
                    let requested_locks = multishard
                        .local_inputs()
                        .iter()
                        .map(|(substate_id, substate)| {
                            VersionedSubstateIdLockIntent::new(
                                VersionedSubstateId::new(substate_id.clone(), substate.version()),
                                SubstateLockType::Write,
                            )
                        })
                        // If outputs are known, lock all local outputs
                        .chain(
                            multishard
                                .outputs()
                                .iter()
                                .filter(|o| local_committee_info.includes_substate_address(&o.to_substate_address()))
                                .map(|output| {
                                    VersionedSubstateIdLockIntent::new(output.clone(), SubstateLockType::Output)
                                }),
                        );
                    store.try_lock_all(transaction_id, requested_locks, false)
                } else {
                    // ABORT - no locks
                    Ok(())
                }
            },
        };

        match lock_result {
            Ok(()) => Ok(prepared),
            Err(err) => {
                let err = err.or_fatal_error()?;
                prepared.set_abort_reason(RejectReason::FailedToLockInputs(err.to_string()));
                Ok(prepared)
            },
        }
    }
}
