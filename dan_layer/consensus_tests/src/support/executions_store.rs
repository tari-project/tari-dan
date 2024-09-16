//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use tari_dan_storage::consensus_models::{Decision, SubstateRequirementLockIntent};
use tari_engine_types::substate::SubstateId;
use tari_transaction::{Transaction, TransactionId};

type TestExecutionOutputMap = HashMap<TransactionId, ExecuteSpec>;

#[derive(Debug, Clone, Default)]
pub struct TestExecutionSpecStore {
    transactions: Arc<RwLock<TestExecutionOutputMap>>,
}

impl TestExecutionSpecStore {
    pub fn new() -> Self {
        Self {
            transactions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn insert(&self, spec: ExecuteSpec) -> &Self {
        self.transactions.write().unwrap().insert(*spec.transaction.id(), spec);
        self
    }

    pub fn get(&self, transaction_id: &TransactionId) -> Option<ExecuteSpec> {
        self.transactions.read().unwrap().get(transaction_id).cloned()
    }
}

#[derive(Debug, Clone)]
pub struct ExecuteSpec {
    pub transaction: Transaction,
    pub decision: Decision,
    pub fee: u64,
    pub inputs: Vec<SubstateRequirementLockIntent>,
    pub new_outputs: Vec<SubstateId>,
}
