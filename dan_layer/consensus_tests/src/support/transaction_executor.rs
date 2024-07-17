//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::{
    hotstuff::substate_store::PendingSubstateStore,
    traits::{BlockTransactionExecutor, BlockTransactionExecutorError},
};
use tari_dan_common_types::{optional::Optional, Epoch};
use tari_dan_storage::{
    consensus_models::{ExecutedTransaction, TransactionRecord},
    StateStore,
};
use tari_transaction::Transaction;

use crate::support::executions_store::TestTransactionExecutionsStore;

#[derive(Debug, Clone)]
pub struct TestBlockTransactionProcessor {
    store: TestTransactionExecutionsStore,
}

impl TestBlockTransactionProcessor {
    pub fn new(store: TestTransactionExecutionsStore) -> Self {
        Self { store }
    }
}

impl<TStateStore: StateStore> BlockTransactionExecutor<TStateStore> for TestBlockTransactionProcessor {
    fn validate(
        &self,
        _tx: &TStateStore::ReadTransaction<'_>,
        _current_epoch: Epoch,
        _transaction: &Transaction,
    ) -> Result<(), BlockTransactionExecutorError> {
        Ok(())
    }

    fn prepare(
        &self,
        transaction: Transaction,
        store: &TStateStore,
    ) -> Result<TransactionRecord, BlockTransactionExecutorError> {
        let t = store.with_read_tx(|tx| TransactionRecord::get(tx, transaction.id()))?;
        Ok(t)
    }

    fn execute(
        &self,
        transaction: Transaction,
        store: &PendingSubstateStore<TStateStore>,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
        if let Some(execution) = self.store.get(transaction.id()) {
            let mut rec = TransactionRecord::new(transaction);
            rec.resolved_inputs = Some(execution.resolved_inputs().to_vec());
            rec.execution_result = Some(execution.result().clone());
            rec.resulting_outputs.clone_from(execution.resulting_outputs());
            rec.execution_time = Some(execution.execution_time());

            return Ok(rec.try_into().unwrap());
        }
        let executed = ExecutedTransaction::get(store.read_transaction(), transaction.id())
            .optional()?
            .expect(
                "ExecutedTransaction was not found by the test executor. Perhaps you need to explicitly add an \
                 execution",
            );
        Ok(executed)
    }
}
