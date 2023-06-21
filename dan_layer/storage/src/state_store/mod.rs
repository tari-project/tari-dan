//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    ops::{Deref, DerefMut},
};

use tari_common_types::types::FixedHash;
use tari_dan_common_types::{Epoch, ShardId};

use crate::{
    consensus_models::{
        Block,
        BlockId,
        ExecutedTransaction,
        HighQc,
        LastExecuted,
        LastVoted,
        LeafBlock,
        LockedBlock,
        PledgeCollection,
        QcId,
        QuorumCertificate,
        TransactionDecision,
        TransactionId,
        TransactionPool,
        Vote,
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
    fn last_voted_get(&mut self, epoch: Epoch) -> Result<LastVoted, StorageError>;
    fn last_executed_get(&mut self, epoch: Epoch) -> Result<LastExecuted, StorageError>;
    fn locked_block_get(&mut self, epoch: Epoch) -> Result<LockedBlock, StorageError>;
    fn leaf_block_get(&mut self, epoch: Epoch) -> Result<LeafBlock, StorageError>;
    fn high_qc_get(&mut self, epoch: Epoch) -> Result<HighQc, StorageError>;
    fn transactions_get(&mut self, tx_id: &TransactionId) -> Result<ExecutedTransaction, StorageError>;
    fn transactions_get_many<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        tx_ids: I,
    ) -> Result<Vec<ExecutedTransaction>, StorageError>;
    fn blocks_get(&mut self, block_id: &BlockId) -> Result<Block, StorageError>;
    fn blocks_exists(&mut self, block_id: &BlockId) -> Result<bool, StorageError>;
    fn blocks_is_ancestor(&mut self, descendant: &BlockId, ancestor: &BlockId) -> Result<bool, StorageError>;
    fn blocks_get_by_parent(&mut self, parent: &BlockId) -> Result<Block, StorageError>;

    fn quorum_certificates_get(&mut self, qc_id: &QcId) -> Result<QuorumCertificate, StorageError>;

    // -------------------------------- Transaction Pools -------------------------------- //
    fn transaction_pools_count(&mut self, pool: TransactionPool) -> Result<usize, StorageError>;
    fn transaction_pools_ready_transaction_count(&mut self) -> Result<usize, StorageError>;
    fn transaction_pools_fetch_involved_shards(
        &mut self,
        transaction_ids: HashSet<TransactionId>,
    ) -> Result<HashSet<ShardId>, StorageError>;

    fn new_transaction_pool_get_specific_decisions(
        &mut self,
        transactions: &BTreeSet<TransactionId>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;

    fn new_transaction_pool_get_many_ready(
        &mut self,
        max_txs: usize,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;

    fn precommitted_transaction_pool_get_many_ready(
        &mut self,
        max_tx: usize,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;
    fn prepared_transaction_pool_get_many_ready(
        &mut self,
        max_txs: usize,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;
    // -------------------------------- Votes -------------------------------- //
    fn votes_get_by_block_and_sender(
        &mut self,
        block_id: &BlockId,
        sender_leaf_hash: &FixedHash,
    ) -> Result<Vote, StorageError>;
    fn votes_count_for_block(&mut self, block_id: &BlockId) -> Result<u64, StorageError>;
    fn votes_get_for_block(&mut self, block_id: &BlockId) -> Result<Vec<Vote>, StorageError>;
}

pub trait StateStoreWriteTransaction {
    fn commit(self) -> Result<(), StorageError>;
    fn rollback(self) -> Result<(), StorageError>;

    // -------------------------------- Block -------------------------------- //
    fn blocks_insert(&mut self, block: &Block) -> Result<(), StorageError>;

    // -------------------------------- QuorumCertificate -------------------------------- //
    fn quorum_certificates_insert(&mut self, qc: &QuorumCertificate) -> Result<(), StorageError>;

    // -------------------------------- Bookkeeping -------------------------------- //
    fn last_voted_set(&mut self, last_voted: &LastVoted) -> Result<(), StorageError>;
    fn last_executed_set(&mut self, last_exec: &LastExecuted) -> Result<(), StorageError>;
    fn leaf_block_set(&mut self, leaf_node: &LeafBlock) -> Result<(), StorageError>;
    fn locked_block_set(&mut self, locked_block: &LockedBlock) -> Result<(), StorageError>;
    fn high_qc_set(&mut self, high_qc: &HighQc) -> Result<(), StorageError>;

    // -------------------------------- Pledges -------------------------------- //
    fn create_pledges(
        &mut self,
        block_id: &BlockId,
        transactions_and_shards: HashMap<TransactionId, Vec<ShardId>>,
    ) -> Result<PledgeCollection, StorageError>;

    // -------------------------------- Transaction Pools -------------------------------- //
    fn transactions_insert(&mut self, executed_transaction: &ExecutedTransaction) -> Result<(), StorageError>;
    fn transactions_mark_many_finalized(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError>;

    // New transaction pool
    fn new_transaction_pool_insert(&mut self, transaction: TransactionDecision) -> Result<(), StorageError>;

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
    ) -> Result<usize, StorageError>;

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
    ) -> Result<usize, StorageError>;

    fn precommitted_transaction_pool_remove_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;

    // Committed transaction pool
    fn committed_transaction_pool_insert_pending(
        &mut self,
        transaction: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError>;
    fn committed_transaction_pool_mark_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<usize, StorageError>;
    fn committed_transaction_pool_remove_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError>;

    // -------------------------------- Votes -------------------------------- //
    fn votes_insert(&mut self, vote: &Vote) -> Result<(), StorageError>;
}
