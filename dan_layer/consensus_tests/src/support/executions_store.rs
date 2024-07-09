//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use tari_dan_storage::consensus_models::TransactionExecution;
use tari_transaction::TransactionId;

#[derive(Debug, Clone, Default)]
pub struct TestTransactionExecutionsStore {
    transactions: Arc<RwLock<HashMap<TransactionId, TransactionExecution>>>,
}

impl TestTransactionExecutionsStore {
    pub fn new() -> Self {
        Self {
            transactions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn insert(&self, execution: TransactionExecution) -> &Self {
        self.transactions
            .write()
            .unwrap()
            .insert(*execution.transaction_id(), execution);
        self
    }

    pub fn get(&self, transaction_id: &TransactionId) -> Option<TransactionExecution> {
        self.transactions.read().unwrap().get(transaction_id).cloned()
    }
}
