//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    ops::DerefMut,
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{optional::Optional, ShardId};
use tari_engine_types::commit_result::{ExecuteResult, FinalizeResult, RejectReason};
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
    final_decision: Option<Decision>,
}

impl ExecutedTransaction {
    pub fn new(transaction: Transaction, result: ExecuteResult) -> Self {
        Self {
            transaction,
            result,
            final_decision: None,
        }
    }

    pub fn new_with_final_decision(
        transaction: Transaction,
        result: ExecuteResult,
        final_decision: Option<Decision>,
    ) -> Self {
        Self {
            transaction,
            result,
            final_decision,
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
