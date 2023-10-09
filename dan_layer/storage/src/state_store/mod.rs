//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    borrow::Borrow,
    collections::HashSet,
    ops::{Deref, DerefMut, RangeInclusive},
};

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{Epoch, NodeAddressable, NodeHeight, ShardId};
use tari_transaction::{Transaction, TransactionId};

use crate::{
    consensus_models::{
        Block,
        BlockId,
        Decision,
        Evidence,
        HighQc,
        LastExecuted,
        LastProposed,
        LastVoted,
        LeafBlock,
        LockedBlock,
        LockedOutput,
        QcId,
        QuorumCertificate,
        SubstateLockFlag,
        SubstateLockState,
        SubstateRecord,
        TransactionAtom,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionPoolStatusUpdate,
        TransactionRecord,
        Vote,
    },
    StorageError,
};

const LOG_TARGET: &str = "tari::dan::storage";

pub trait StateStore {
    type Addr: NodeAddressable;
    type ReadTransaction<'a>: StateStoreReadTransaction<Addr = Self::Addr>
    where Self: 'a;
    type WriteTransaction<'a>: StateStoreWriteTransaction<Addr = Self::Addr>
        + Deref<Target = Self::ReadTransaction<'a>>
        + DerefMut
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
    type Addr: NodeAddressable;

    fn last_voted_get(&mut self) -> Result<LastVoted, StorageError>;
    fn last_executed_get(&mut self) -> Result<LastExecuted, StorageError>;
    fn last_proposed_get(&mut self) -> Result<LastProposed, StorageError>;
    fn locked_block_get(&mut self) -> Result<LockedBlock, StorageError>;
    fn leaf_block_get(&mut self) -> Result<LeafBlock, StorageError>;
    fn high_qc_get(&mut self) -> Result<HighQc, StorageError>;
    fn transactions_get(&mut self, tx_id: &TransactionId) -> Result<TransactionRecord, StorageError>;
    fn transactions_exists(&mut self, tx_id: &TransactionId) -> Result<bool, StorageError>;

    fn transactions_get_any<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        tx_ids: I,
    ) -> Result<Vec<TransactionRecord>, StorageError>;
    fn transactions_get_paginated(
        &mut self,
        limit: u64,
        offset: u64,
        asc_desc_created_at: Option<Ordering>,
    ) -> Result<Vec<TransactionRecord>, StorageError>;
    fn blocks_get(&mut self, block_id: &BlockId) -> Result<Block<Self::Addr>, StorageError>;
    fn blocks_get_tip(&mut self) -> Result<Block<Self::Addr>, StorageError>;
    fn blocks_all_after(&mut self, block_id: &BlockId) -> Result<Vec<Block<Self::Addr>>, StorageError>;
    fn blocks_exists(&mut self, block_id: &BlockId) -> Result<bool, StorageError>;
    fn blocks_is_ancestor(&mut self, descendant: &BlockId, ancestor: &BlockId) -> Result<bool, StorageError>;
    fn blocks_get_all_by_parent(&mut self, parent: &BlockId) -> Result<Vec<Block<Self::Addr>>, StorageError>;
    fn blocks_get_parent_chain(
        &mut self,
        block_id: &BlockId,
        limit: usize,
    ) -> Result<Vec<Block<Self::Addr>>, StorageError>;
    fn blocks_get_pending_transactions(&mut self, block_id: &BlockId) -> Result<Vec<TransactionId>, StorageError>;
    fn blocks_get_total_leader_fee_for_epoch(
        &mut self,
        epoch: Epoch,
        validator_public_key: &Self::Addr,
    ) -> Result<u64, StorageError>;
    fn blocks_get_any_with_epoch_range(
        &mut self,
        epoch_range: RangeInclusive<Epoch>,
        validator_public_key: Option<&Self::Addr>,
    ) -> Result<Vec<Block<Self::Addr>>, StorageError>;
    fn blocks_get_paginated(
        &mut self,
        limit: u64,
        offset: u64,
        asc_desc_created_at: Option<Ordering>,
    ) -> Result<Vec<Block<Self::Addr>>, StorageError>;
    fn blocks_get_count(&mut self) -> Result<i64, StorageError>;

    fn blocks_max_height(&mut self) -> Result<NodeHeight, StorageError>;

    fn parked_blocks_exists(&mut self, block_id: &BlockId) -> Result<bool, StorageError>;

    // -------------------------------- QuorumCertificate -------------------------------- //
    fn quorum_certificates_get(&mut self, qc_id: &QcId) -> Result<QuorumCertificate<Self::Addr>, StorageError>;
    fn quorum_certificates_get_all<'a, I: IntoIterator<Item = &'a QcId>>(
        &mut self,
        qc_ids: I,
    ) -> Result<Vec<QuorumCertificate<Self::Addr>>, StorageError>;
    fn quorum_certificates_get_by_block_id(
        &mut self,
        block_id: &BlockId,
    ) -> Result<QuorumCertificate<Self::Addr>, StorageError>;

    // -------------------------------- Transaction Pools -------------------------------- //
    fn transaction_pool_get(
        &mut self,
        from_block_id: &BlockId,
        to_block_id: &BlockId,
        transaction_id: &TransactionId,
    ) -> Result<TransactionPoolRecord, StorageError>;
    fn transaction_pool_exists(&mut self, transaction_id: &TransactionId) -> Result<bool, StorageError>;
    fn transaction_pool_get_many_ready(&mut self, max_txs: usize) -> Result<Vec<TransactionPoolRecord>, StorageError>;
    fn transaction_pool_count(
        &mut self,
        stage: Option<TransactionPoolStage>,
        is_ready: Option<bool>,
        has_foreign_data: Option<bool>,
    ) -> Result<usize, StorageError>;

    fn transactions_fetch_involved_shards(
        &mut self,
        transaction_ids: HashSet<TransactionId>,
    ) -> Result<HashSet<ShardId>, StorageError>;

    // -------------------------------- Votes -------------------------------- //
    fn votes_get_by_block_and_sender(
        &mut self,
        block_id: &BlockId,
        sender_leaf_hash: &FixedHash,
    ) -> Result<Vote<Self::Addr>, StorageError>;
    fn votes_count_for_block(&mut self, block_id: &BlockId) -> Result<u64, StorageError>;
    fn votes_get_for_block(&mut self, block_id: &BlockId) -> Result<Vec<Vote<Self::Addr>>, StorageError>;
    //---------------------------------- Substates --------------------------------------------//
    fn substates_get(&mut self, substate_id: &ShardId) -> Result<SubstateRecord, StorageError>;
    fn substates_get_any(&mut self, substate_ids: &HashSet<ShardId>) -> Result<Vec<SubstateRecord>, StorageError>;
    fn substates_any_exist<I, S>(&mut self, substates: I) -> Result<bool, StorageError>
    where
        I: IntoIterator<Item = S>,
        S: Borrow<ShardId>;

    fn substates_exists_for_transaction(&mut self, transaction_id: &TransactionId) -> Result<bool, StorageError>;

    fn substates_get_many_within_range(
        &mut self,
        start: &ShardId,
        end: &ShardId,
        exclude_shards: &[ShardId],
    ) -> Result<Vec<SubstateRecord>, StorageError>;
    fn substates_get_many_by_created_transaction(
        &mut self,
        tx_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError>;

    fn substates_get_many_by_destroyed_transaction(
        &mut self,
        tx_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError>;
    fn substates_get_all_for_block(&mut self, block_id: &BlockId) -> Result<Vec<SubstateRecord>, StorageError>;
    fn substates_get_all_for_transaction(
        &mut self,
        transaction_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError>;
    fn substates_check_lock_many<'a, I: IntoIterator<Item = &'a ShardId>>(
        &mut self,
        objects: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<SubstateLockState, StorageError>;

    fn locked_outputs_check_all<I, B>(&mut self, output_shards: I) -> Result<SubstateLockState, StorageError>
    where
        I: IntoIterator<Item = B>,
        B: Borrow<ShardId>;
}

pub trait StateStoreWriteTransaction {
    type Addr: NodeAddressable;

    fn commit(self) -> Result<(), StorageError>;
    fn rollback(self) -> Result<(), StorageError>;

    // -------------------------------- Block -------------------------------- //
    fn blocks_insert(&mut self, block: &Block<Self::Addr>) -> Result<(), StorageError>;
    fn blocks_set_flags(
        &mut self,
        block_id: &BlockId,
        is_committed: Option<bool>,
        is_processed: Option<bool>,
    ) -> Result<(), StorageError>;

    // -------------------------------- QuorumCertificate -------------------------------- //
    fn quorum_certificates_insert(&mut self, qc: &QuorumCertificate<Self::Addr>) -> Result<(), StorageError>;

    // -------------------------------- Bookkeeping -------------------------------- //
    fn last_voted_set(&mut self, last_voted: &LastVoted) -> Result<(), StorageError>;
    fn last_votes_unset(&mut self, last_voted: &LastVoted) -> Result<(), StorageError>;
    fn last_executed_set(&mut self, last_exec: &LastExecuted) -> Result<(), StorageError>;
    fn last_proposed_set(&mut self, last_proposed: &LastProposed) -> Result<(), StorageError>;
    fn last_proposed_unset(&mut self, last_proposed: &LastProposed) -> Result<(), StorageError>;
    fn leaf_block_set(&mut self, leaf_node: &LeafBlock) -> Result<(), StorageError>;
    fn locked_block_set(&mut self, locked_block: &LockedBlock) -> Result<(), StorageError>;
    fn high_qc_set(&mut self, high_qc: &HighQc) -> Result<(), StorageError>;

    // -------------------------------- Transaction -------------------------------- //
    fn transactions_insert(&mut self, transaction: &Transaction) -> Result<(), StorageError>;
    fn transactions_update(&mut self, transaction: &TransactionRecord) -> Result<(), StorageError>;
    fn transactions_save_all<'a, I: IntoIterator<Item = &'a TransactionRecord>>(
        &mut self,
        transaction: I,
    ) -> Result<(), StorageError>;
    // -------------------------------- Transaction Pool -------------------------------- //
    fn transaction_pool_insert(
        &mut self,
        transaction: TransactionAtom,
        stage: TransactionPoolStage,
        is_ready: bool,
    ) -> Result<(), StorageError>;
    fn transaction_pool_add_pending_update(
        &mut self,
        pool_update: TransactionPoolStatusUpdate,
    ) -> Result<(), StorageError>;

    fn transaction_pool_update(
        &mut self,
        transaction_id: &TransactionId,
        local_decision: Option<Decision>,
        remote_decision: Option<Decision>,
        remote_evidence: Option<&Evidence>,
    ) -> Result<(), StorageError>;
    fn transaction_pool_remove(&mut self, transaction_id: &TransactionId) -> Result<(), StorageError>;
    fn transaction_pool_set_all_transitions<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        locked_block: &LockedBlock,
        new_locked_block: &LockedBlock,
        tx_ids: I,
    ) -> Result<(), StorageError>;

    fn missing_transactions_insert<
        'a,
        IMissing: IntoIterator<Item = &'a TransactionId>,
        IAwaiting: IntoIterator<Item = &'a TransactionId>,
    >(
        &mut self,
        park_block: &Block<Self::Addr>,
        missing_transaction_ids: IMissing,
        awaiting_transaction_ids: IAwaiting,
    ) -> Result<(), StorageError>;

    fn missing_transactions_remove(
        &mut self,
        current_height: NodeHeight,
        transaction_id: TransactionId,
    ) -> Result<Option<Block<Self::Addr>>, StorageError>;

    // -------------------------------- Votes -------------------------------- //
    fn votes_insert(&mut self, vote: &Vote<Self::Addr>) -> Result<(), StorageError>;

    //---------------------------------- Substates --------------------------------------------//
    fn substates_try_lock_many<'a, I: IntoIterator<Item = &'a ShardId>>(
        &mut self,
        locked_by_tx: &TransactionId,
        objects: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<SubstateLockState, StorageError>;

    fn substates_try_unlock_many<'a, I: IntoIterator<Item = &'a ShardId>>(
        &mut self,
        locked_by_tx: &TransactionId,
        objects: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<(), StorageError>;

    fn substate_down_many<I: IntoIterator<Item = ShardId>>(
        &mut self,
        shard_ids: I,
        epoch: Epoch,
        destroyed_block_id: &BlockId,
        destroyed_transaction_id: &TransactionId,
        destroyed_qc_id: &QcId,
        require_locks: bool,
    ) -> Result<(), StorageError>;
    fn substates_create(&mut self, substate: SubstateRecord) -> Result<(), StorageError>;
    // -------------------------------- Locked Outputs -------------------------------- //
    fn locked_outputs_acquire_all<I, B>(
        &mut self,
        block_id: &BlockId,
        transaction_id: &TransactionId,
        output_shards: I,
    ) -> Result<SubstateLockState, StorageError>
    where
        I: IntoIterator<Item = B>,
        B: Borrow<ShardId>;

    fn locked_outputs_release_all<I, B>(&mut self, output_shards: I) -> Result<Vec<LockedOutput>, StorageError>
    where
        I: IntoIterator<Item = B>,
        B: Borrow<ShardId>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Ordering {
    Ascending,
    Descending,
}
