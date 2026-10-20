//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use tari_consensus_types::Decision;
use tari_engine_types::{ValidatorFeeWithdrawal, published_template::PublishedTemplateAddress, substate::SubstateId};
use tari_ootle_common_types::SubstateLockType;
use tari_ootle_transaction::{Transaction, TransactionId};

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
        self.transactions
            .write()
            .unwrap()
            .insert(spec.transaction.calculate_id(), spec);
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
    pub input_locks: Vec<(SubstateId, SubstateLockType)>,
    pub new_outputs: Vec<SubstateId>,
    pub validator_fee_withdrawals: Vec<ValidatorFeeWithdrawal>,
    /// A template the transaction calls. Execution aborts unless the parent chain of the block it executes in
    /// publishes the template.
    pub calls_template: Option<PublishedTemplateAddress>,
}
