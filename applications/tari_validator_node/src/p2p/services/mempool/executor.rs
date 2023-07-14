//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::time::{Duration, Instant};

use log::*;
use tari_dan_app_utilities::transaction_executor::{TransactionExecutor, TransactionProcessorError};
use tari_dan_engine::{
    bootstrap_state,
    runtime::ConsensusContext,
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

pub(super) type ExecutionResult = (TransactionId, Duration, Result<ExecutedTransaction, MempoolError>);

pub async fn execute_transaction<TSubstateResolver, TExecutor>(
    transaction: Transaction,
    substate_resolver: TSubstateResolver,
    executor: TExecutor,
    consensus_context: ConsensusContext,
) -> Result<ExecutionResult, MempoolError>
where
    TSubstateResolver: SubstateResolver<Error = SubstateResolverError>,
    TExecutor: TransactionExecutor<Error = TransactionProcessorError> + Send + Sync + 'static,
{
    let mut state_db = new_state_db();

    let timer = Instant::now();
    match substate_resolver.resolve(&transaction, &mut state_db).await {
        Ok(()) => {
            let res = task::spawn_blocking(move || {
                let id = *transaction.id();
                let result = executor.execute(transaction, state_db, consensus_context);
                (id, timer.elapsed(), result.map_err(MempoolError::from))
            })
            .await;

            // If this errors, the thread panicked due to a bug
            res.map_err(|err| MempoolError::ExecutionThreadFailure(err.to_string()))
        },
        // Substates are downed/dont exist
        Err(err @ SubstateResolverError::InputSubstateDowned { .. }) |
        Err(err @ SubstateResolverError::InputSubstateDoesNotExist { .. }) => {
            warn!(target: LOG_TARGET, "Invalid input shards for transaction {}: {}", transaction.id(), err);
            Ok((*transaction.id(), Duration::default(), Err(err.into())))
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
