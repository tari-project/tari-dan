//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_consensus::traits::TransactionExecutor;
use tari_dan_app_utilities::transaction_executor::TransactionProcessorError;
use tari_dan_common_types::Epoch;
use tari_dan_engine::{
    bootstrap_state,
    state_store::{memory::MemoryStateStore, AtomicDb, StateWriter},
};
use tari_dan_storage::consensus_models::ExecutedTransaction;
use tari_transaction::{Transaction, TransactionId};
use tokio::task;

use crate::{
    p2p::services::mempool::{MempoolError, SubstateResolver},
    substate_resolver::SubstateResolverError,
};

const LOG_TARGET: &str = "tari::dan::mempool::executor";

pub(super) type ExecutionResult = (TransactionId, Result<ExecutedTransaction, MempoolError>);

pub async fn execute_transaction<TSubstateResolver, TExecutor>(
    transaction: Transaction,
    substate_resolver: TSubstateResolver,
    executor: TExecutor,
    current_epoch: Epoch,
) -> Result<ExecutionResult, MempoolError>
where
    TSubstateResolver: SubstateResolver<Error = SubstateResolverError>,
    TExecutor: TransactionExecutor<Error = TransactionProcessorError> + Send + Sync + 'static,
{
    let id = *transaction.id();

    let state_db = new_state_db();
    let virtual_substates = match substate_resolver
        .resolve_virtual_substates(&transaction, current_epoch)
        .await
    {
        Ok(virtual_substates) => virtual_substates,
        Err(err @ SubstateResolverError::UnauthorizedFeeClaim { .. }) => {
            warn!(target: LOG_TARGET, "One or more invalid fee claims for transaction {}: {}", transaction.id(), err);
            return Ok((*transaction.id(), Err(err.into())));
        },
        Err(err) => return Err(err.into()),
    };

    info!(target: LOG_TARGET, "Transaction {} executing. virtual_substates = [{}]", transaction.id(), virtual_substates.keys().map(|addr| addr.to_string()).collect::<Vec<_>>().join(", "));

    match substate_resolver.resolve(&transaction, &state_db).await {
        Ok(()) => {
            let res = task::spawn_blocking(move || {
                let result = executor.execute(transaction, state_db, virtual_substates);
                (id, result.map_err(MempoolError::from))
            })
            .await;

            // If this errors, the thread panicked due to a bug
            res.map_err(|err| MempoolError::ExecutionThreadFailure(err.to_string()))
        },
        // Substates are downed/dont exist
        Err(err @ SubstateResolverError::InputSubstateDowned { .. }) |
        Err(err @ SubstateResolverError::InputSubstateDoesNotExist { .. }) => {
            warn!(target: LOG_TARGET, "One or more invalid input shards for transaction {}: {}", transaction.id(), err);
            Ok((*transaction.id(), Err(err.into())))
        },
        // Some other issue - network, db, etc
        Err(err) => Err(err.into()),
    }
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
