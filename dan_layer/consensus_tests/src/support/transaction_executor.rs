//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_consensus::traits::TransactionExecutor;
use tari_dan_app_utilities::transaction_executor::TransactionProcessorError;
use tari_dan_engine::state_store::memory::MemoryStateStore;
use tari_dan_storage::consensus_models::ExecutedTransaction;
use tari_engine_types::{commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult}, virtual_substate::VirtualSubstates};
use tari_template_lib::Hash;
use tari_transaction::Transaction;

#[derive(Debug, Clone)]
pub struct TestTransactionProcessor {}

impl TestTransactionProcessor {
    pub fn new() -> Self {
        Self { }
    }
}

impl TransactionExecutor for TestTransactionProcessor
{
    type Error = TransactionProcessorError;

    fn execute(
        &self,
        transaction: Transaction,
        state_store: MemoryStateStore,
        virtual_substates: VirtualSubstates,
    ) -> Result<ExecutedTransaction, Self::Error> {
        let outputs = vec![];
        let result = ExecuteResult {
            finalize: FinalizeResult {
                transaction_hash: Hash::default(),
                events: vec![],
                logs: vec![],
                execution_results: vec![],
                result: TransactionResult::Reject(RejectReason::ExecutionFailure("TestTransactionProcessor is just a mock".to_owned())),
                cost_breakdown: None,
            },
            fee_receipt: None,
        };
        let execution_time = Duration::ZERO;
        Ok(ExecutedTransaction::new(transaction, result, outputs, execution_time))
    }
}
