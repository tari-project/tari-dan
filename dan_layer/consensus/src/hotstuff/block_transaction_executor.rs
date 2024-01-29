//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::{info, warn};
use tari_transaction::Transaction;
use tokio::task;

use crate::traits::ConsensusSpec;

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::block_transaction_executor";

#[derive(thiserror::Error, Debug)]
pub enum BlockTransactionExecutorError {
    #[error("Placeholder error")]
    PlaceHolderError,
}

// TODO: we should keep a "proxy" hashmap (or memory storage) of updated states from previous transactions in the same block,
//       so each consecutive transaction gets the most updated version for their inputs.
//       If no previous tx wrote on a input, just get it from the regular state store
#[derive(Debug, Clone)]
pub struct BlockTransactionExecutor<TConsensusSpec: ConsensusSpec, TExecutor> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    executor: TExecutor,
}

impl<TConsensusSpec, TExecutor> BlockTransactionExecutor<TConsensusSpec, TExecutor>
where
    TConsensusSpec: ConsensusSpec,
    TExecutor: TransactionExecutor<Error = TransactionProcessorError> + Send + Sync + 'static
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        executor: TExecutor,
    ) -> Self {
        Self {
           store,
           epoch_manager,
           executor
        }
    }

    pub async fn execute(
        &self,
        transaction: &Transaction,
    ) -> Result<(), BlockTransactionExecutorError> {
        let id = *transaction.id();

        info!(
            target: LOG_TARGET,
            "Executing transaction: {}",
            id,
        );     

        let state_db = self.new_state_db();
        let virtual_substates = match self
            .resolve_virtual_substates(&transaction)
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

        match self.resolve_substates(&transaction, &state_db).await {
            Ok(()) => {
                let res = task::spawn_blocking(move || {
                    let result = self.executor.execute(transaction, state_db, virtual_substates);
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

    fn new_state_db(&self) -> MemoryStateStore {
        let state_db = MemoryStateStore::new();
        // unwrap: Memory state store is infallible
        let mut tx = state_db.write_access().unwrap();
        // Add bootstrapped substates
        bootstrap_state(&mut tx).unwrap();
        tx.commit().unwrap();
        state_db
    }

    async fn resolve_substates(&self, transaction: &Transaction, out: &MemoryStateStore) {
        todo!()
    }

    async fn resolve_virtual_substates(&self, transaction: &Transaction) {
        todo!()
    }
}

