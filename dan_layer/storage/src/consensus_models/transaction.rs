//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    ops::DerefMut,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::ShardId;
use tari_engine_types::commit_result::{ExecuteResult, FinalizeResult, RejectReason};
use tari_transaction::{Transaction, TransactionId};

use crate::{
    consensus_models::{Decision, ExecutedTransaction},
    Ordering,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRecord {
    pub transaction: Transaction,
    pub result: Option<ExecuteResult>,
    pub execution_time: Option<Duration>,
    pub final_decision: Option<Decision>,
    pub abort_details: Option<String>,
}

impl TransactionRecord {
    pub fn new(transaction: Transaction) -> Self {
        Self {
            transaction,
            result: None,
            execution_time: None,
            final_decision: None,
            abort_details: None,
        }
    }

    pub fn new_with_details(
        transaction: Transaction,
        result: Option<ExecuteResult>,
        execution_time: Option<Duration>,
        final_decision: Option<Decision>,
        abort_details: Option<String>,
    ) -> Self {
        Self {
            transaction,
            result,
            execution_time,
            final_decision,
            abort_details,
        }
    }

    pub fn transaction(&self) -> &Transaction {
        &self.transaction
    }

    pub fn transaction_mut(&mut self) -> &mut Transaction {
        &mut self.transaction
    }

    pub fn into_transaction(self) -> Transaction {
        self.transaction
    }

    pub fn result(&self) -> Option<&ExecuteResult> {
        self.result.as_ref()
    }

    pub fn final_decision(&self) -> Option<Decision> {
        self.final_decision
    }

    pub fn execution_time(&self) -> Option<Duration> {
        self.execution_time
    }

    pub fn abort_details(&self) -> Option<&String> {
        self.abort_details.as_ref()
    }

    pub fn set_abort<T: Into<String>>(&mut self, details: T) -> &mut Self {
        self.final_decision = Some(Decision::Abort);
        self.abort_details = Some(details.into());
        self
    }

    pub fn into_final_result(self) -> Option<ExecuteResult> {
        self.final_decision().and_then(|d| {
            if d.is_commit() {
                self.result
            } else {
                Some(ExecuteResult {
                    finalize: FinalizeResult::reject(
                        self.transaction.id().into_array().into(),
                        RejectReason::ShardRejected(format!(
                            "Validators decided to abort: {}",
                            self.abort_details
                                .as_deref()
                                .unwrap_or("<invalid state, no abort details>")
                        )),
                    ),
                    transaction_failure: None,
                    fee_receipt: None,
                })
            }
        })
    }
}

impl TransactionRecord {
    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.transactions_insert(&self.transaction)
    }

    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        if !Self::exists(tx.deref_mut(), self.transaction.id())? {
            self.insert(tx)?;
        }
        Ok(())
    }

    pub fn update<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.transactions_update(self)
    }

    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx, tx_id: &TransactionId) -> Result<Self, StorageError> {
        tx.transactions_get(tx_id)
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &mut TTx,
        tx_id: &TransactionId,
    ) -> Result<bool, StorageError> {
        tx.transactions_exists(tx_id)
    }

    pub fn get_any<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a TransactionId>>(
        tx: &mut TTx,
        tx_ids: I,
    ) -> Result<Vec<Self>, StorageError> {
        tx.transactions_get_any(tx_ids)
    }

    pub fn get_paginated<TTx: StateStoreReadTransaction>(
        tx: &mut TTx,
        limit: u64,
        offset: u64,
        ordering: Option<Ordering>,
    ) -> Result<Vec<Self>, StorageError> {
        tx.transactions_get_paginated(limit, offset, ordering)
    }

    pub fn get_involved_shards<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a TransactionId>>(
        tx: &mut TTx,
        transactions: I,
    ) -> Result<HashMap<TransactionId, HashSet<ShardId>>, StorageError> {
        let transactions = Self::get_any(tx, transactions)?;
        Ok(transactions
            .into_iter()
            .map(|t| {
                (
                    *t.transaction.id(),
                    t.transaction.involved_shards_iter().copied().collect(),
                )
            })
            .collect())
    }
}

impl From<ExecutedTransaction> for TransactionRecord {
    fn from(tx: ExecutedTransaction) -> Self {
        let execution_time = tx.execution_time();
        let final_decision = tx.final_decision();
        let abort_details = tx.abort_details().cloned();
        let (transaction, result) = tx.dissolve();
        Self {
            transaction,
            result: Some(result),
            execution_time: Some(execution_time),
            final_decision,
            abort_details,
        }
    }
}
