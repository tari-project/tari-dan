//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashSet},
    ops::DerefMut,
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::ShardId;

use crate::{
    consensus_models::{TransactionDecision, TransactionId},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Copy)]
pub enum TransactionPool {
    New,
    Prepare,
    Precommit,
    Commit,
}

#[derive(Debug, Clone, Copy)]
pub struct AllTransactionPools;

impl AllTransactionPools {
    pub fn has_ready_transactions<TTx: StateStoreReadTransaction>(tx: &mut TTx) -> Result<bool, StorageError> {
        Ok(tx.transaction_pools_ready_transaction_count()? > 0)
    }

    pub fn find_involved_shards<TTx, I>(tx: &mut TTx, transaction_ids: I) -> Result<HashSet<ShardId>, StorageError>
    where
        TTx: DerefMut,
        TTx::Target: StateStoreReadTransaction,
        I: Iterator<Item = TransactionId>,
    {
        tx.deref_mut()
            .transaction_pools_fetch_involved_shards(transaction_ids.collect())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NewTransactionPool;

impl NewTransactionPool {
    pub fn insert<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        transaction_decision: TransactionDecision,
    ) -> Result<(), StorageError> {
        tx.new_transaction_pool_insert(transaction_decision)
    }

    pub fn move_specific_to_prepare<TTx>(
        tx: &mut TTx,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>
    where
        TTx: StateStoreWriteTransaction + DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        let ready_txs = tx.new_transaction_pool_remove_specific_ready(transactions)?;
        tx.prepared_transaction_pool_insert_pending(&ready_txs)?;
        Ok(ready_txs)
    }

    pub fn all_decisions_match<TTx: StateStoreReadTransaction>(
        tx: &mut TTx,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<bool, StorageError> {
        let decisions =
            tx.new_transaction_pool_get_specific_decisions(&transactions.iter().map(|t| t.transaction_id).collect())?;
        Ok(decisions == *transactions)
    }

    pub fn move_many_to_prepare<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        max_txs: usize,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError> {
        if max_txs == 0 {
            return Ok(BTreeSet::new());
        }
        let ready_txs = tx.new_transaction_pool_remove_many_ready(max_txs)?;
        tx.prepared_transaction_pool_insert_pending(&ready_txs)?;
        Ok(ready_txs)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PrepareTransactionPool;

impl PrepareTransactionPool {
    pub fn mark_specific_ready<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError> {
        tx.prepared_transaction_pool_mark_specific_ready(transactions)?;
        Ok(())
    }

    pub fn move_specific_to_precommit<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError> {
        let ready_txs = tx.prepared_transaction_pool_remove_specific_ready(transactions)?;
        tx.precommitted_transaction_pool_insert_pending(&ready_txs)?;
        Ok(ready_txs)
    }

    pub fn move_many_to_precommit<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        max_txs: usize,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError> {
        if max_txs == 0 {
            return Ok(BTreeSet::new());
        }
        let ready_txs = tx.prepared_transaction_pool_remove_many_ready(max_txs)?;
        tx.precommitted_transaction_pool_insert_pending(&ready_txs)?;
        Ok(ready_txs)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PrecommitTransactionPool;

impl PrecommitTransactionPool {
    pub fn mark_specific_ready<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError> {
        tx.precommitted_transaction_pool_mark_specific_ready(transactions)?;
        Ok(())
    }

    pub fn move_specific_to_committed<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError> {
        let ready_txs = tx.precommitted_transaction_pool_remove_specific_ready(transactions)?;
        tx.committed_transaction_pool_insert_pending(&ready_txs)?;
        Ok(ready_txs)
    }

    pub fn move_many_to_committed<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        max_txs: usize,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError> {
        if max_txs == 0 {
            return Ok(BTreeSet::new());
        }
        let ready_txs = tx.precommitted_transaction_pool_remove_many_ready(max_txs)?;
        tx.committed_transaction_pool_insert_pending(&ready_txs)?;
        Ok(ready_txs)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CommittedTransactionPool;

impl CommittedTransactionPool {
    pub fn mark_specific_ready<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError> {
        tx.committed_transaction_pool_mark_specific_ready(transactions)?;
        Ok(())
    }

    pub fn finalize_specific<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError> {
        tx.committed_transaction_pool_remove_specific_ready(transactions)?;
        tx.transactions_mark_many_finalized(transactions)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionReadiness {
    Ready,
    Pending,
}
