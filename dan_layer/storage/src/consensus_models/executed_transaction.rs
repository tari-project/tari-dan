//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    ops::DerefMut,
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{optional::Optional, ShardId};
use tari_engine_types::commit_result::ExecuteResult;
use tari_transaction::{Transaction, TransactionId};

use crate::{
    consensus_models::{Decision, Evidence},
    Ordering,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutedTransaction {
    transaction: Transaction,
    result: ExecuteResult,
    is_finalized: bool,
}

impl ExecutedTransaction {
    pub fn new(transaction: Transaction, result: ExecuteResult) -> Self {
        Self {
            transaction,
            result,
            is_finalized: false,
        }
    }

    pub fn new_with_finalized(transaction: Transaction, result: ExecuteResult, is_finalized: bool) -> Self {
        Self {
            transaction,
            result,
            is_finalized,
        }
    }

    pub fn as_decision(&self) -> Decision {
        if self.result.finalize.is_accept() {
            Decision::Commit
        } else {
            Decision::Abort
        }
    }

    pub fn transaction_decision(&self) -> Decision {
        if self.result.transaction_failure.is_none() {
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

    pub fn into_result(self) -> ExecuteResult {
        self.result
    }

    pub fn to_initial_evidence(&self) -> Evidence {
        self.transaction
            .involved_shards_iter()
            .map(|shard| (*shard, vec![]))
            .collect()
    }

    pub fn is_finalized(&self) -> bool {
        self.is_finalized
    }

    pub fn set_as_finalized(&mut self) -> &mut Self {
        self.is_finalized = true;
        self
    }
}

impl ExecutedTransaction {
    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.transactions_insert(self)
    }

    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        if Self::exists(tx.deref_mut(), self.transaction.id())? {
            self.update(tx)
        } else {
            self.insert(tx)
        }
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
        // TODO: optimise
        let t = tx.transactions_get(tx_id).optional()?;
        Ok(t.is_some())
    }

    pub fn get_many<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a TransactionId>>(
        tx: &mut TTx,
        tx_ids: I,
    ) -> Result<Vec<Self>, StorageError> {
        tx.transactions_get_many(tx_ids)
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
        let transactions = Self::get_many(tx, transactions)?;
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
