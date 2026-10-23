//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Speculative execution of a block's transactions ahead of its evaluation in block order.
//!
//! Evaluating a block runs in three phases. Phase 0 applies what precedes the block's transactions, such as its
//! foreign proposals, to the block's store. Phase 1, here, executes the transactions that need only local inputs in
//! parallel, each against the store as phase 0 left it plus the changes of the transactions before it in its
//! [`ExecutionUnit`](super::ExecutionUnit), and records each execution in an [`ExecutionMemo`]. Phase 2 is the
//! evaluation itself, in block order, which takes an execution from the memo only where the transaction resolves to the
//! inputs it was speculated with, and executes it otherwise. Every consensus rule is decided in phase 2, so a
//! speculation that read state the block does not have costs the time it took and changes no result.

use std::collections::HashMap;

use log::*;
use tari_engine_types::substate::{Substate, SubstateId};
use tari_ootle_common_types::{
    Epoch,
    SubstateRequirement,
    SubstateVersion,
    committee::CommitteeInfo,
    optional::IsNotFoundError,
};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    consensus_models::{LockedEpoch, TransactionExecution, TransactionRecord},
};
use tari_ootle_transaction::{Transaction, TransactionId};

use super::{
    ConsensusTransactionManager,
    DeclaredAccess,
    ExecutionMemo,
    ExecutionPlan,
    ExecutionRunner,
    manager::{declared_lock_intent, input_fanout},
};
use crate::{
    hotstuff::substate_store::{PendingSubstateStore, SubstateStoreError},
    tracing::TraceTimer,
    traits::{BlockTransactionExecutor, UncommittedChainTemplates, WriteableSubstateStore},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::speculation";

/// A transaction to speculate, and the epoch its evaluation will pin it to.
#[derive(Debug, Clone)]
pub struct SpeculationCandidate {
    pub transaction_id: TransactionId,
    pub locked_epoch: LockedEpoch,
}

/// The threads a node speculates on, and the store it reads.
#[derive(Debug, Clone)]
pub(super) struct Speculation<TStateStore> {
    runner: ExecutionRunner,
    /// Speculation reads a snapshot of its own. A block is evaluated in a write transaction, whose reads cannot be
    /// shared across threads.
    state_store: TStateStore,
}

impl<TStateStore> Speculation<TStateStore> {
    /// Speculation on `execution_threads` threads, or `None` with fewer than two.
    pub fn new(execution_threads: usize, state_store: TStateStore) -> Option<Self> {
        (execution_threads > 1).then(|| Self {
            runner: ExecutionRunner::new(execution_threads),
            state_store,
        })
    }
}

struct Speculated {
    transaction_id: TransactionId,
    inputs: HashMap<SubstateRequirement, Substate>,
    locked_epoch: LockedEpoch,
    execution: TransactionExecution,
}

impl<TStateStore, TExecutor> ConsensusTransactionManager<TExecutor, TStateStore>
where
    TStateStore: StateStore + Sync,
    TExecutor: BlockTransactionExecutor<TStateStore> + Sync,
{
    /// Executes those of `candidates` that a block of `block_epoch` executes against its local inputs alone, against
    /// the state `store` holds, and returns the executions for the block's evaluation to take. `candidates` are in the
    /// order the block evaluates them.
    ///
    /// The state is read from a snapshot of the store's chain, with the changes `store` already holds applied on top.
    /// A snapshot that lags the transaction `store` reads in only makes speculations miss.
    ///
    /// Speculation never fails: a transaction it cannot execute is left for the evaluation to execute.
    pub fn speculate<TTx: StateStoreReadTransaction>(
        &self,
        store: &PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        block_epoch: Epoch,
        candidates: &[SpeculationCandidate],
    ) -> ExecutionMemo {
        let mut memo = ExecutionMemo::new(*store.parent_block().block_id());
        let Some(speculation) = self.speculation() else {
            return memo;
        };
        if candidates.is_empty() {
            return memo;
        }
        let timer = TraceTimer::info(LOG_TARGET, "Speculate executions").with_iterations(candidates.len());

        // The templates are those the evaluation executes with, read from the chain `store` builds on
        let templates = match self.uncommitted_templates().templates_for(store) {
            Ok(templates) => templates,
            Err(err) => {
                warn!(target: LOG_TARGET, "⚠️ Not speculating: the templates of the chain could not be read: {err}");
                return memo;
            },
        };
        let snapshot = match speculation.state_store.create_read_tx() {
            Ok(snapshot) => snapshot,
            Err(err) => {
                warn!(target: LOG_TARGET, "⚠️ Not speculating: no snapshot of the store: {err}");
                return memo;
            },
        };
        let base = match snapshot_of(&snapshot, store, local_committee_info) {
            Ok(base) => base,
            Err(err) => {
                warn!(target: LOG_TARGET, "⚠️ Not speculating: the block's state could not be read from a snapshot: {err}");
                return memo;
            },
        };

        let transactions = speculation
            .runner
            .map(candidates, |candidate| {
                TransactionRecord::get(&snapshot, &candidate.transaction_id)
                    .inspect_err(|err| {
                        debug!(
                            target: LOG_TARGET,
                            "Not speculating transaction {}: {err}",
                            candidate.transaction_id
                        )
                    })
                    .ok()
                    .map(|record| (record, candidate.locked_epoch.clone()))
            })
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let plan = ExecutionPlan::new(
            &transactions
                .iter()
                .map(|(record, _)| declared_access(record.transaction()))
                .collect::<Vec<_>>(),
        );

        let speculated = speculation.runner.map(plan.units(), |unit| {
            self.speculate_unit(
                &base,
                local_committee_info,
                block_epoch,
                &templates,
                unit.members().iter().map(|idx| &transactions[*idx]),
            )
        });
        for Speculated {
            transaction_id,
            inputs,
            locked_epoch,
            execution,
        } in speculated.into_iter().flatten()
        {
            memo.insert(transaction_id, &inputs, locked_epoch, execution);
        }

        let longest = plan.units().iter().max_by_key(|unit| unit.members().len());
        info!(
            target: LOG_TARGET,
            "⚡ Speculated {} of {} transaction(s) in {} unit(s) on {} thread(s). Longest unit: {} transaction(s), \
             weight {}",
            memo.num_untaken(),
            candidates.len(),
            plan.units().len(),
            speculation.runner.num_threads(),
            longest.map_or(0, |unit| unit.members().len()),
            longest.map_or(0, |unit| unit.weight()),
        );
        timer.done();
        memo
    }

    /// Speculates `transactions` one after another, each against the changes of those before it. A transaction whose
    /// changes do not apply means the unit read state the block will not have, so the transactions after it are not
    /// speculated.
    fn speculate_unit<'a, TTx: StateStoreReadTransaction>(
        &self,
        store: &PendingSubstateStore<TTx>,
        local_committee_info: &CommitteeInfo,
        block_epoch: Epoch,
        templates: &UncommittedChainTemplates,
        transactions: impl Iterator<Item = &'a (TransactionRecord, LockedEpoch)>,
    ) -> Vec<Speculated> {
        let mut overlay = store.overlay();
        let mut speculated = Vec::new();
        for (record, locked_epoch) in transactions {
            let transaction = record.transaction();
            if self
                .execution_abort_reason(transaction, locked_epoch.epoch(), block_epoch)
                .is_some()
            {
                continue;
            }
            let resolved = match self.resolve_local_versions(&overlay, local_committee_info, transaction) {
                Ok(resolved) => resolved,
                // The evaluation aborts a transaction with a missing input, which changes no substate
                Err(err) if err.is_not_found_error() => continue,
                Err(err) => {
                    debug!(target: LOG_TARGET, "Stopped speculating a unit at transaction {}: {err}", record.id());
                    break;
                },
            };
            let is_executed_locally = resolved.foreign_inputs.is_empty() &&
                !(transaction.is_global() && local_committee_info.num_committees() > 1);
            if !is_executed_locally {
                continue;
            }

            let inputs = match overlay.get_many(
                resolved
                    .local_inputs
                    .iter()
                    .map(|(decl, version)| (decl.to_substate_requirement_ref().to_owned(), *version)),
            ) {
                Ok(inputs) => inputs,
                Err(err) => {
                    debug!(target: LOG_TARGET, "Stopped speculating a unit at transaction {}: {err}", record.id());
                    break;
                },
            };
            let execution = match self.executor().execute(
                transaction,
                locked_epoch.clone(),
                input_fanout(transaction, local_committee_info),
                &inputs,
                templates,
            ) {
                Ok(execution) => execution,
                Err(err) => {
                    debug!(target: LOG_TARGET, "Stopped speculating a unit at transaction {}: {err}", record.id());
                    break;
                },
            };

            // A commit changes the block's state when all its outputs are local. A cross-shard one takes locks.
            if execution.decision().is_commit() &&
                local_committee_info.is_all_local(execution.resulting_outputs()) &&
                let Some(diff) = execution.result().finalize.any_accept() &&
                let Err(err) = overlay.put_diff(diff)
            {
                debug!(
                    target: LOG_TARGET,
                    "Stopped speculating a unit at transaction {}, whose changes do not apply: {err}",
                    record.id()
                );
                break;
            }

            speculated.push(Speculated {
                transaction_id: *record.id(),
                inputs,
                locked_epoch: locked_epoch.clone(),
                execution,
            });
        }
        speculated
    }
}

/// The state `store` holds, read through `snapshot`.
fn snapshot_of<'a, TSnapshot: StateStoreReadTransaction, TTx: StateStoreReadTransaction>(
    snapshot: &'a TSnapshot,
    store: &PendingSubstateStore<TTx>,
    local_committee_info: &CommitteeInfo,
) -> Result<PendingSubstateStore<'a, TSnapshot>, SubstateStoreError> {
    let mut base = PendingSubstateStore::init(snapshot, store.parent_block(), local_committee_info.num_preshards())?;
    for change in store.changes() {
        base.put(change.clone())?;
    }
    Ok(base)
}

/// The substates `transaction` declares it touches. Its receipt is left out, because no other transaction touches it.
fn declared_access(transaction: &Transaction) -> DeclaredAccess {
    let inputs = transaction.all_inputs_iter().map(|decl| {
        let is_write = declared_lock_intent(&decl, SubstateVersion::ZERO)
            .lock_type()
            .is_write();
        (decl.substate_id().clone(), is_write)
    });
    let tombstones = transaction
        .claim_burn_outputs_iter()
        .map(|tombstone| (SubstateId::from(tombstone), true));
    DeclaredAccess {
        weight: transaction.calculate_transaction_weight().as_u64(),
        substates: inputs.chain(tombstones).collect(),
    }
}
