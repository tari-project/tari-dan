//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_app_utilities::transaction_executor::{TransactionExecutor, TransactionProcessorError};
use tari_dan_common_types::Epoch;
use tari_dan_engine::{
    bootstrap_state,
    state_store::{memory::MemoryStateStore, AtomicDb, StateWriter},
};
use tari_dan_storage::consensus_models::{ExecutedTransaction, SubstateLockFlag, VersionedSubstateIdLockIntent};
use tari_transaction::{Transaction, VersionedSubstateId};
use tokio::task;

use crate::{
    p2p::services::mempool::{MempoolError, ResolvedSubstates, SubstateResolver},
    substate_resolver::SubstateResolverError,
};

const LOG_TARGET: &str = "tari::dan::mempool::executor";

pub async fn execute_transaction<TSubstateResolver, TExecutor>(
    transaction: Transaction,
    substate_resolver: TSubstateResolver,
    executor: TExecutor,
    current_epoch: Epoch,
) -> Result<Result<ExecutedTransaction, MempoolError>, MempoolError>
where
    TSubstateResolver: SubstateResolver<Error = SubstateResolverError>,
    TExecutor: TransactionExecutor<Error = TransactionProcessorError> + Send + Sync + 'static,
{
    let virtual_substates = match substate_resolver
        .resolve_virtual_substates(&transaction, current_epoch)
        .await
    {
        Ok(virtual_substates) => virtual_substates,
        Err(err @ SubstateResolverError::UnauthorizedFeeClaim { .. }) => {
            warn!(target: LOG_TARGET, "One or more invalid fee claims for transaction {}: {}", transaction.id(), err);
            return Ok(Err(err.into()));
        },
        Err(err) => return Err(err.into()),
    };

    info!(target: LOG_TARGET, "🎱 Transaction {} found virtual_substates = [{}]", transaction.id(), virtual_substates.keys().map(|addr| addr.to_string()).collect::<Vec<_>>().join(", "));

    let ResolvedSubstates {
        local: local_substates,
        unresolved_foreign: foreign,
    } = match substate_resolver.try_resolve_local(&transaction) {
        Ok(pair) => pair,
        // Substates are downed/dont exist
        Err(err @ SubstateResolverError::InputSubstateDowned { .. }) |
        Err(err @ SubstateResolverError::InputSubstateDoesNotExist { .. }) => {
            warn!(target: LOG_TARGET, "One or more invalid input shards for transaction {}: {}", transaction.id(), err);
            // Ok(Err(_)) return that the transaction should be rejected, not an internal mempool execution failure
            return Ok(Err(err.into()));
        },
        // Some other issue - network, db, etc
        Err(err) => return Err(err.into()),
    };

    if !foreign.is_empty() {
        info!(target: LOG_TARGET, "Unable to execute transaction {} in the mempool because it has foreign inputs: {:?}", transaction.id(), foreign);
        return Err(MempoolError::MustDeferExecution {
            local_substates,
            foreign_substates: foreign,
        });
    }

    info!(target: LOG_TARGET, "🎱 Transaction {} resolved local inputs = [{}]", transaction.id(), local_substates.keys().map(|addr| addr.to_string()).collect::<Vec<_>>().join(", "));

    let res = task::spawn_blocking(move || {
        let versioned_inputs = local_substates
            .iter()
            .map(|(id, substate)| VersionedSubstateId::new(id.clone(), substate.version()))
            .collect::<Vec<_>>();
        let state_db = new_state_db();
        state_db.set_many(local_substates).expect("memory db is infallible");

        match executor.execute(transaction, state_db, virtual_substates) {
            Ok(exec_output) => {
                // Update the resolved inputs to set the specific version, as we know it after execution
                let resolved_inputs = if let Some(diff) = exec_output.result.finalize.accept() {
                    versioned_inputs
                        .into_iter()
                        .map(|versioned_id| {
                            let lock_flag = if diff.down_iter().any(|(id, _)| *id == versioned_id.substate_id) {
                                // Update all inputs that were DOWNed to be write locked
                                SubstateLockFlag::Write
                            } else {
                                // Any input not downed, gets a read lock
                                SubstateLockFlag::Read
                            };
                            VersionedSubstateIdLockIntent::new(versioned_id, lock_flag)
                        })
                        .collect()
                } else {
                    versioned_inputs
                        .into_iter()
                        .map(|versioned_id| {
                            // We cannot tell which inputs are written, however since this transaction is a
                            // reject it does not matter since it will not cause locks.
                            // We still set resolved inputs because this is used to determine which shards are
                            // involved.
                            VersionedSubstateIdLockIntent::new(versioned_id, SubstateLockFlag::Write)
                        })
                        .collect()
                };

                Ok(ExecutedTransaction::new(
                    exec_output.transaction,
                    exec_output.result,
                    resolved_inputs,
                    exec_output.outputs,
                    exec_output.execution_time,
                ))
            },
            Err(err) => Err(err.into()),
        }
    })
    .await;

    // If this errors, the thread panicked due to a bug
    res.map_err(|err| MempoolError::ExecutionThreadPanicked(err.to_string()))
}

fn new_state_db() -> MemoryStateStore {
    let state_db = MemoryStateStore::new();
    // unwrap: Memory state store is infallible
    let mut tx = state_db.write_access().unwrap();
    // Add bootstrapped substates
    bootstrap_state(&mut tx).unwrap();
    tx.commit().unwrap();
    state_db
}
