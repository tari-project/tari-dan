//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use tari_dan_storage::consensus_models::BlockTransactionExecution;
use tari_transaction::TransactionId;

type TestExecutionStore = HashMap<TransactionId, BlockTransactionExecution>;

#[derive(Debug, Clone, Default)]
pub struct TestTransactionExecutionsStore {
    transactions: Arc<RwLock<TestExecutionStore>>,
}

impl TestTransactionExecutionsStore {
    pub fn new() -> Self {
        Self {
            transactions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn insert(&self, execution: BlockTransactionExecution) -> &Self {
        self.transactions
            .write()
            .unwrap()
            .insert(*execution.transaction_id(), execution);
        self
    }

    pub fn get(&self, transaction_id: &TransactionId) -> Option<BlockTransactionExecution> {
        self.transactions.read().unwrap().get(transaction_id).cloned()
    }
}
