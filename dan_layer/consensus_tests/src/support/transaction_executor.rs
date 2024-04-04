//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::{
    BlockTransactionExecutor,
    BlockTransactionExecutorBuilder,
    BlockTransactionExecutorError,
};
use tari_dan_storage::{consensus_models::ExecutedTransaction, StateStore};
use tari_transaction::Transaction;

#[derive(Debug, Clone)]
pub struct TestBlockTransactionExecutorBuilder {}

impl TestBlockTransactionExecutorBuilder {
    pub fn new() -> Self {
        Self {}
    }
}

impl<TStateStore> BlockTransactionExecutorBuilder<TStateStore> for TestBlockTransactionExecutorBuilder
where TStateStore: StateStore
{
    fn build(&self) -> Box<dyn BlockTransactionExecutor<TStateStore>> {
        Box::new(TestBlockTransactionProcessor::new())
    }
}

#[derive(Debug, Clone)]
pub struct TestBlockTransactionProcessor {}

impl TestBlockTransactionProcessor {
    pub fn new() -> Self {
        Self {}
    }
}

impl<TStateStore> BlockTransactionExecutor<TStateStore> for TestBlockTransactionProcessor
where TStateStore: StateStore
{
    fn execute(
        &mut self,
        transaction: Transaction,
        db_tx: &mut TStateStore::ReadTransaction<'_>,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
        // Tests generate executed transactions, so if execute is called we expect it to already be in the database.
        let executed = ExecutedTransaction::get(db_tx, transaction.id())?;
        Ok(executed)
    }
}
