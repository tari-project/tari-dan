//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    ops::DerefMut,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::ShardId;
use tari_engine_types::commit_result::ExecuteResult;
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
}

impl TransactionRecord {
    pub fn new(transaction: Transaction) -> Self {
        Self {
            transaction,
            result: None,
            execution_time: None,
            final_decision: None,
        }
    }

    pub fn new_with_details(
        transaction: Transaction,
        result: Option<ExecuteResult>,
        execution_time: Option<Duration>,
        final_decision: Option<Decision>,
    ) -> Self {
        Self {
            transaction,
            result,
            execution_time,
            final_decision,
        }
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
        let (transaction, result) = tx.dissolve();
        Self {
            transaction,
            result: Some(result),
            execution_time: Some(execution_time),
            final_decision,
        }
    }
}
