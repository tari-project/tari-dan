//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashSet},
    ops::{Deref, DerefMut},
};

use tari_common_types::types::FixedHash;
use tari_dan_common_types::{Epoch, NodeHeight, ShardId};

use crate::{
    consensus_models::{
        Block,
        BlockId,
        ExecutedTransaction,
        HighQc,
        LeafBlock,
        QuorumCertificate,
        Transaction,
        TransactionDecision,
        TransactionId,
    },
    StorageError,
};

const LOG_TARGET: &str = "tari::dan::storage";

pub trait StateStore {
    type ReadTransaction<'a>: StateStoreReadTransaction
    where Self: 'a;
    type WriteTransaction<'a>: StateStoreWriteTransaction + Deref<Target = Self::ReadTransaction<'a>> + DerefMut
    where Self: 'a;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, StorageError>;
    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, StorageError>;

    fn with_write_tx<F: FnOnce(&mut Self::WriteTransaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where E: From<StorageError> {
        let mut tx = self.create_write_tx()?;
        match f(&mut tx) {
            Ok(r) => {
                tx.commit()?;
                Ok(r)
            },
            Err(e) => {
                if let Err(err) = tx.rollback() {
                    log::error!(target: LOG_TARGET, "Failed to rollback transaction: {}", err);
                }
                Err(e)
            },
        }
    }

    fn with_read_tx<F: FnOnce(&mut Self::ReadTransaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where E: From<StorageError> {
        let mut tx = self.create_read_tx()?;
        let ret = f(&mut tx)?;
        Ok(ret)
    }
}

pub trait StateStoreReadTransaction {
    fn last_vote_height_get(&mut self, epoch: Epoch) -> Result<u64, StorageError>;
    fn locked_block_get(&mut self, epoch: Epoch) -> Result<(NodeHeight, FixedHash), StorageError>;
    fn leaf_block_get(&mut self, epoch: Epoch) -> Result<LeafBlock, StorageError>;
    fn high_qc_get(&mut self, epoch: Epoch) -> Result<HighQc, StorageError>;
    fn transactions_get(&mut self, tx_id: &TransactionId) -> Result<Transaction, StorageError>;
    fn blocks_get(&mut self, block_id: &BlockId) -> Result<Block, StorageError>;
    fn quorum_certificates_get(&mut self, block_id: &BlockId) -> Result<QuorumCertificate, StorageError>;

    // -------------------------------- Transaction Pools -------------------------------- //
    fn transaction_pools_ready_transaction_count(&mut self) -> Result<usize, StorageError>;
    fn transaction_pools_fetch_involved_shards(
        &mut self,
        transaction_ids: HashSet<TransactionId>,
    ) -> Result<HashSet<ShardId>, StorageError>;
}

pub trait StateStoreWriteTransaction {
    fn commit(self) -> Result<(), StorageError>;
    fn rollback(self) -> Result<(), StorageError>;

    // -------------------------------- Block -------------------------------- //
    fn blocks_insert(&mut self, block: &Block) -> Result<(), StorageError>;

    fn leaf_block_set(&mut self, leaf_node: &LeafBlock) -> Result<(), StorageError>;
    fn high_qc_set(&mut self, high_qc: &HighQc) -> Result<(), StorageError>;

    // -------------------------------- Transaction Pools -------------------------------- //
    fn transactions_insert(&mut self, executed_transaction: &ExecutedTransaction) -> Result<(), StorageError>;
    fn transactions_mark_many_finalized(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError>;

    // New transaction pool
    fn new_transaction_pool_insert(&mut self, transaction: TransactionDecision) -> Result<(), StorageError>;
    /// Removes up to max_tx transactions from the new transaction pool and returns them
    fn new_transaction_pool_remove_many_ready(
        &mut self,
        max_txs: usize,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;
    fn new_transaction_pool_remove_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;

    // Prepared transaction pool
    fn prepared_transaction_pool_insert_pending(
        &mut self,
        transaction: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError>;

    fn prepared_transaction_pool_mark_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError>;

    fn prepared_transaction_pool_remove_many_ready(
        &mut self,
        max_txs: usize,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;
    fn prepared_transaction_pool_remove_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;

    // Precommitted transaction pool
    fn precommitted_transaction_pool_insert_pending(
        &mut self,
        transaction: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError>;

    fn precommitted_transaction_pool_mark_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError>;

    fn precommitted_transaction_pool_remove_many_ready(
        &mut self,
        max_txs: usize,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;

    fn precommitted_transaction_pool_remove_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;

    // Committed transaction pool
    fn committed_transaction_pool_insert_pending(
        &mut self,
        transaction: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError>;
    fn committed_transaction_pool_mark_many_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError>;
    fn committed_transaction_pool_remove_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;
}
