//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::info;
use tari_dan_engine::{
    bootstrap_state,
    state_store::{memory::MemoryStateStore, AtomicDb, StateWriter},
};
use tari_engine_types::virtual_substate::VirtualSubstates;
use tari_transaction::Transaction;
use tokio::task;

use crate::traits::{ConsensusSpec, TransactionExecutor};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::block_transaction_executor";

// TODO: more refined errors
#[derive(thiserror::Error, Debug)]
pub enum BlockTransactionExecutorError {
    #[error("Placeholder error")]
    PlaceHolderError,
    #[error("Execution thread failure: {0}")]
    ExecutionThreadFailure(String),
}

// TODO: we should keep a "proxy" hashmap (or memory storage) of updated states from previous transactions in the same
// block,       so each consecutive transaction gets the most updated version for their inputs.
//       If no previous tx wrote on a input, just get it from the regular state store
#[derive(Debug, Clone)]
pub struct BlockTransactionExecutor<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    executor: TConsensusSpec::TransactionExecutor,
}

impl<TConsensusSpec> BlockTransactionExecutor<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        executor: TConsensusSpec::TransactionExecutor,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            executor,
        }
    }

    pub fn execute(&self, transaction: Transaction) -> Result<(), BlockTransactionExecutorError> {
        let id: tari_transaction::TransactionId = *transaction.id();

        info!(
            target: LOG_TARGET,
            "Executing transaction: {}",
            id,
        );

        let state_db = self.new_state_db();
        let virtual_substates = match self.resolve_virtual_substates(&transaction) {
            Ok(virtual_substates) => virtual_substates,
            Err(err) => return Err(err.into()),
        };

        info!(target: LOG_TARGET, "Transaction {} executing. virtual_substates = [{}]", transaction.id(), virtual_substates.keys().map(|addr| addr.to_string()).collect::<Vec<_>>().join(", "));
        let executor = self.executor.clone();
        let _result = match self.resolve_substates(&transaction, &state_db) {
            Ok(()) => {
                // TODO: proper error variant
                let result = executor
                    .execute(transaction, state_db, virtual_substates)
                    .map_err(|_| BlockTransactionExecutorError::PlaceHolderError);

                // If this errors, the thread panicked due to a bug
                result.map_err(|err| BlockTransactionExecutorError::ExecutionThreadFailure(err.to_string()))
            },
            Err(err) => Err(err.into()),
        };

        Ok(())
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

    fn resolve_substates(
        &self,
        _transaction: &Transaction,
        _out: &MemoryStateStore,
    ) -> Result<(), BlockTransactionExecutorError> {
        // TODO
        Ok(())
    }

    fn resolve_virtual_substates(
        &self,
        _transaction: &Transaction,
    ) -> Result<VirtualSubstates, BlockTransactionExecutorError> {
        // TODO
        Ok(VirtualSubstates::new())
    }
}
