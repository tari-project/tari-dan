//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use indexmap::IndexMap;
use tari_consensus::traits::{BlockTransactionExecutor, BlockTransactionExecutorError};
use tari_dan_common_types::{Epoch, SubstateRequirement};
use tari_dan_storage::{
    consensus_models::{ExecutedTransaction, TransactionRecord},
    StateStore,
};
use tari_engine_types::substate::Substate;
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

    fn execute(
        &self,
        transaction: Transaction,
        _current_epoch: Epoch,
        _resolved_inputs: &IndexMap<SubstateRequirement, Substate>,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
        let execution = self.store.get(transaction.id()).unwrap_or_else(|| {
            panic!(
                "Missing execution for transaction {} to TestTransactionExecutionsStore",
                transaction.id()
            )
        });
        let mut rec = TransactionRecord::new(transaction);
        rec.resolved_inputs = Some(execution.resolved_inputs().to_vec());
        rec.execution_result = Some(execution.result().clone());
        rec.resulting_outputs = Some(execution.resulting_outputs().to_vec());

        Ok(rec.try_into().unwrap())
        // let executed = ExecutedTransaction::get(store.read_transaction(), transaction.id())
        //     .optional()?
        //     .expect(
        //         "ExecutedTransaction was not found by the test executor. Perhaps you need to explicitly add an \
        //          execution",
        //     );
        // Ok(executed)
    }
}
