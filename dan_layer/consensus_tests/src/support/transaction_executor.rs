//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_consensus::traits::{
    BlockTransactionExecutor,
    BlockTransactionExecutorBuilder,
    BlockTransactionExecutorError,
};
use tari_dan_storage::{consensus_models::ExecutedTransaction, StateStore};
use tari_engine_types::commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult};
use tari_template_lib::Hash;
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
        return Box::new(TestBlockTransactionProcessor::new());
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
        _db_tx: &mut TStateStore::ReadTransaction<'_>,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
        let outputs = vec![];
        let result = ExecuteResult {
            finalize: FinalizeResult {
                transaction_hash: Hash::default(),
                events: vec![],
                logs: vec![],
                execution_results: vec![],
                result: TransactionResult::Reject(RejectReason::ExecutionFailure(
                    "TestBlockTransactionProcessor is just a mock".to_owned(),
                )),
                cost_breakdown: None,
            },
            fee_receipt: None,
        };
        let execution_time = Duration::ZERO;
        Ok(ExecutedTransaction::new(transaction, result, outputs, execution_time))
    }
}
