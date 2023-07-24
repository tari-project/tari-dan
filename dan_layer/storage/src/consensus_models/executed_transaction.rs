//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    ops::DerefMut,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{optional::Optional, ShardId};
use tari_engine_types::commit_result::{ExecuteResult, FinalizeResult, RejectReason};
use tari_transaction::{Transaction, TransactionId};

use crate::{
    consensus_models::{Decision, Evidence, TransactionRecord},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutedTransaction {
    transaction: Transaction,
    result: ExecuteResult,
    execution_time: Duration,
    final_decision: Option<Decision>,
}

impl ExecutedTransaction {
    pub fn new(transaction: Transaction, result: ExecuteResult, execution_time: Duration) -> Self {
        Self {
            transaction,
            result,
            execution_time,
            final_decision: None,
        }
    }

    pub fn new_with_final_decision(
        transaction: Transaction,
        result: ExecuteResult,
        execution_time: Duration,
        final_decision: Option<Decision>,
    ) -> Self {
        Self {
            transaction,
            result,
            execution_time,
            final_decision,
        }
    }

    pub fn as_decision(&self) -> Decision {
        if self.result.finalize.is_accept() {
            Decision::Commit
        } else {
            Decision::Abort
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

    pub fn result(&self) -> &ExecuteResult {
        &self.result
    }

    pub fn into_final_result(self) -> Option<ExecuteResult> {
        self.final_decision().map(|d| {
            if d.is_commit() {
                self.result
            } else {
                // TODO: We preserve the original result mainly for debugging purposes, but this is a little hacky
                ExecuteResult {
                    finalize: FinalizeResult::reject(
                        self.result.finalize.transaction_hash,
                        RejectReason::ShardRejected("Validators decided to abort".to_string()),
                    ),
                    transaction_failure: None,
                    fee_receipt: None,
                }
            }
        })
    }

    pub fn into_result(self) -> ExecuteResult {
        self.result
    }

    pub fn execution_time(&self) -> Duration {
        self.execution_time
    }

    pub fn dissolve(self) -> (Transaction, ExecuteResult) {
        (self.transaction, self.result)
    }

    pub fn to_initial_evidence(&self) -> Evidence {
        self.transaction
            .involved_shards_iter()
            .map(|shard| (*shard, vec![]))
            .collect()
    }

    pub fn is_finalized(&self) -> bool {
        self.final_decision.is_some()
    }

    pub fn final_decision(&self) -> Option<Decision> {
        self.final_decision
    }

    pub fn set_final_decision(&mut self, decision: Decision) -> &mut Self {
        self.final_decision = Some(decision);
        self
    }
}

impl ExecutedTransaction {
    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.transactions_insert(self.transaction())?;
        tx.executed_transactions_update(self)
    }

    pub fn upsert<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        if TransactionRecord::exists(tx.deref_mut(), self.transaction.id())? {
            self.update(tx)
        } else {
            self.insert(tx)
        }
    }

    pub fn update<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.executed_transactions_update(self)
    }

    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx, tx_id: &TransactionId) -> Result<Self, StorageError> {
        let rec = tx.transactions_get(tx_id)?;
        if rec.result.is_none() {
            return Err(StorageError::NotFound {
                item: "ExecutedTransaction".to_string(),
                key: tx_id.to_string(),
            });
        }

        // This should never fail as we just checked that the transaction has been executed
        rec.try_into()
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<bool, StorageError> {
        match tx.transactions_get(self.transaction.id()).optional()? {
            Some(rec) => Ok(rec.result.is_some()),
            None => Ok(false),
        }
    }

    pub fn get_any<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a TransactionId>>(
        tx: &mut TTx,
        tx_ids: I,
    ) -> Result<Vec<Self>, StorageError> {
        let recs = tx.transactions_get_any(tx_ids)?;
        recs.into_iter().map(|rec| rec.try_into()).collect()
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

impl TryFrom<TransactionRecord> for ExecutedTransaction {
    type Error = StorageError;

    fn try_from(value: TransactionRecord) -> Result<Self, Self::Error> {
        if value.result.is_none() {
            return Err(StorageError::QueryError {
                reason: format!("Transaction {} has not yet executed", value.transaction.id()),
            });
        }

        Ok(Self {
            transaction: value.transaction,
            result: value.result.unwrap(),
            execution_time: value.execution_time.unwrap_or_default(),
            is_finalized: value.is_finalized,
        })
    }
}
