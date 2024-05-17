//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    borrow::Borrow,
    collections::HashSet,
    ops::{Deref, RangeInclusive},
};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::{Epoch, NodeAddressable, NodeHeight, SubstateAddress};
use tari_engine_types::substate::SubstateId;
use tari_state_tree::{TreeStore, TreeStoreReader, Version};
use tari_transaction::{SubstateRequirement, TransactionId, VersionedSubstateId};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    consensus_models::{
        Block,
        BlockDiff,
        BlockId,
        Decision,
        Evidence,
        ForeignProposal,
        ForeignReceiveCounters,
        ForeignSendCounters,
        HighQc,
        LastExecuted,
        LastProposed,
        LastSentVote,
        LastVoted,
        LeafBlock,
        LockedBlock,
        LockedSubstate,
        PendingStateTreeDiff,
        QcId,
        QuorumCertificate,
        SubstateRecord,
        TransactionAtom,
        TransactionExecution,
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
    type ReadTransaction<'a>: StateStoreReadTransaction<Addr = Self::Addr> + TreeStoreReader<Version>
    where Self: 'a;
    type WriteTransaction<'a>: StateStoreWriteTransaction<Addr = Self::Addr>
        + TreeStore<Version>
        + Deref<Target = Self::ReadTransaction<'a>>
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

    fn with_read_tx<F: FnOnce(&Self::ReadTransaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where E: From<StorageError> {
        let tx = self.create_read_tx()?;
        let ret = f(&tx)?;
        Ok(ret)
    }
}

pub trait StateStoreReadTransaction: Sized {
    type Addr: NodeAddressable;
    fn last_sent_vote_get(&self) -> Result<LastSentVote, StorageError>;
    fn last_voted_get(&self) -> Result<LastVoted, StorageError>;
    fn last_executed_get(&self) -> Result<LastExecuted, StorageError>;
    fn last_proposed_get(&self) -> Result<LastProposed, StorageError>;
    fn locked_block_get(&self) -> Result<LockedBlock, StorageError>;
    fn leaf_block_get(&self) -> Result<LeafBlock, StorageError>;
    fn high_qc_get(&self) -> Result<HighQc, StorageError>;
    fn foreign_proposal_exists(&self, foreign_proposal: &ForeignProposal) -> Result<bool, StorageError>;
    fn foreign_proposal_get_all_new(&self) -> Result<Vec<ForeignProposal>, StorageError>;
    fn foreign_proposal_get_all_pending(
        &self,
        from_block_id: &BlockId,
        to_block_id: &BlockId,
    ) -> Result<Vec<ForeignProposal>, StorageError>;
    fn foreign_proposal_get_all_proposed(&self, to_height: NodeHeight) -> Result<Vec<ForeignProposal>, StorageError>;
    fn foreign_send_counters_get(&self, block_id: &BlockId) -> Result<ForeignSendCounters, StorageError>;
    fn foreign_receive_counters_get(&self) -> Result<ForeignReceiveCounters, StorageError>;
    fn transactions_get(&self, tx_id: &TransactionId) -> Result<TransactionRecord, StorageError>;
    fn transactions_exists(&self, tx_id: &TransactionId) -> Result<bool, StorageError>;

    fn transactions_get_any<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &self,
        tx_ids: I,
    ) -> Result<Vec<TransactionRecord>, StorageError>;
    fn transactions_get_paginated(
        &self,
        limit: u64,
        offset: u64,
        asc_desc_created_at: Option<Ordering>,
    ) -> Result<Vec<TransactionRecord>, StorageError>;

    fn transaction_executions_get(
        &self,
        tx_id: &TransactionId,
        block: &BlockId,
    ) -> Result<TransactionExecution, StorageError>;

    fn transaction_executions_get_pending_for_block(
        &self,
        tx_id: &TransactionId,
        from_block_id: &BlockId,
    ) -> Result<TransactionExecution, StorageError>;
    fn blocks_get(&self, block_id: &BlockId) -> Result<Block, StorageError>;
    fn blocks_get_tip(&self) -> Result<Block, StorageError>;
    /// Returns all blocks from and excluding the start block (lower height) to the end block (inclusive)
    fn blocks_get_all_between(
        &self,
        start_block_id_exclusive: &BlockId,
        end_block_id_inclusive: &BlockId,
        include_dummy_blocks: bool,
    ) -> Result<Vec<Block>, StorageError>;
    fn blocks_exists(&self, block_id: &BlockId) -> Result<bool, StorageError>;
    fn blocks_is_ancestor(&self, descendant: &BlockId, ancestor: &BlockId) -> Result<bool, StorageError>;
    fn blocks_get_all_by_parent(&self, parent: &BlockId) -> Result<Vec<Block>, StorageError>;
    fn blocks_get_parent_chain(&self, block_id: &BlockId, limit: usize) -> Result<Vec<Block>, StorageError>;
    fn blocks_get_pending_transactions(&self, block_id: &BlockId) -> Result<Vec<TransactionId>, StorageError>;
    fn blocks_get_total_leader_fee_for_epoch(
        &self,
        epoch: Epoch,
        validator_public_key: &PublicKey,
    ) -> Result<u64, StorageError>;
    fn blocks_get_any_with_epoch_range(
        &self,
        epoch_range: RangeInclusive<Epoch>,
        validator_public_key: Option<&PublicKey>,
    ) -> Result<Vec<Block>, StorageError>;
    fn blocks_get_paginated(
        &self,
        limit: u64,
        offset: u64,
        filter_index: Option<usize>,
        filter: Option<String>,
        ordering_index: Option<usize>,
        ordering: Option<Ordering>,
    ) -> Result<Vec<Block>, StorageError>;
    fn blocks_get_count(&self) -> Result<i64, StorageError>;

    fn filtered_blocks_get_count(
        &self,
        filter_index: Option<usize>,
        filter: Option<String>,
    ) -> Result<i64, StorageError>;
    fn blocks_max_height(&self) -> Result<NodeHeight, StorageError>;

    fn block_diffs_get(&self, block_id: &BlockId) -> Result<BlockDiff, StorageError>;

    fn parked_blocks_exists(&self, block_id: &BlockId) -> Result<bool, StorageError>;

    // -------------------------------- QuorumCertificate -------------------------------- //
    fn quorum_certificates_get(&self, qc_id: &QcId) -> Result<QuorumCertificate, StorageError>;
    fn quorum_certificates_get_all<'a, I: IntoIterator<Item = &'a QcId>>(
        &self,
        qc_ids: I,
    ) -> Result<Vec<QuorumCertificate>, StorageError>;
    fn quorum_certificates_get_by_block_id(&self, block_id: &BlockId) -> Result<QuorumCertificate, StorageError>;

    // -------------------------------- Transaction Pools -------------------------------- //
    fn transaction_pool_get(&self, transaction_id: &TransactionId) -> Result<TransactionPoolRecord, StorageError>;
    fn transaction_pool_get_for_blocks(
        &self,
        from_block_id: &BlockId,
        to_block_id: &BlockId,
        transaction_id: &TransactionId,
    ) -> Result<TransactionPoolRecord, StorageError>;
    fn transaction_pool_exists(&self, transaction_id: &TransactionId) -> Result<bool, StorageError>;
    fn transaction_pool_get_all(&self) -> Result<Vec<TransactionPoolRecord>, StorageError>;
    fn transaction_pool_get_many_ready(&self, max_txs: usize) -> Result<Vec<TransactionPoolRecord>, StorageError>;
    fn transaction_pool_count(
        &self,
        stage: Option<TransactionPoolStage>,
        is_ready: Option<bool>,
        has_foreign_data: Option<bool>,
    ) -> Result<usize, StorageError>;

    fn transactions_fetch_involved_shards(
        &self,
        transaction_ids: HashSet<TransactionId>,
    ) -> Result<HashSet<SubstateAddress>, StorageError>;

    // -------------------------------- Votes -------------------------------- //
    fn votes_get_by_block_and_sender(
        &self,
        block_id: &BlockId,
        sender_leaf_hash: &FixedHash,
    ) -> Result<Vote, StorageError>;
    fn votes_count_for_block(&self, block_id: &BlockId) -> Result<u64, StorageError>;
    fn votes_get_for_block(&self, block_id: &BlockId) -> Result<Vec<Vote>, StorageError>;
    //---------------------------------- Substates --------------------------------------------//
    fn substates_get(&self, substate_id: &SubstateAddress) -> Result<SubstateRecord, StorageError>;
    fn substates_get_any(
        &self,
        substate_ids: &HashSet<SubstateRequirement>,
    ) -> Result<Vec<SubstateRecord>, StorageError>;
    fn substates_get_any_max_version<'a, I: IntoIterator<Item = &'a SubstateId>>(
        &self,
        substate_ids: I,
    ) -> Result<Vec<SubstateRecord>, StorageError>;
    fn substates_any_exist<I, S>(&self, substates: I) -> Result<bool, StorageError>
    where
        I: IntoIterator<Item = S>,
        S: Borrow<VersionedSubstateId>;

    fn substates_exists_for_transaction(&self, transaction_id: &TransactionId) -> Result<bool, StorageError>;

    fn substates_get_many_within_range(
        &self,
        start: &SubstateAddress,
        end: &SubstateAddress,
        exclude_shards: &[SubstateAddress],
    ) -> Result<Vec<SubstateRecord>, StorageError>;
    fn substates_get_many_by_created_transaction(
        &self,
        tx_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError>;

    fn substates_get_many_by_destroyed_transaction(
        &self,
        tx_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError>;
    fn substates_get_all_for_block(&self, block_id: &BlockId) -> Result<Vec<SubstateRecord>, StorageError>;
    fn substates_get_all_for_transaction(
        &self,
        transaction_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError>;

    fn substate_locks_get_all_for_block(
        &self,
        block_id: BlockId,
    ) -> Result<IndexMap<SubstateId, Vec<LockedSubstate>>, StorageError>;

    fn substate_locks_get_latest_for_substate(&self, substate_id: &SubstateId) -> Result<LockedSubstate, StorageError>;

    fn pending_state_tree_diffs_exists_for_block(&self, block_id: &BlockId) -> Result<bool, StorageError>;
    fn pending_state_tree_diffs_get_all_up_to_commit_block(
        &self,
        block_id: &BlockId,
    ) -> Result<Vec<PendingStateTreeDiff>, StorageError>;
}

pub trait StateStoreWriteTransaction {
    type Addr: NodeAddressable;

    fn commit(self) -> Result<(), StorageError>;
    fn rollback(self) -> Result<(), StorageError>;

    // -------------------------------- Block -------------------------------- //
    fn blocks_insert(&mut self, block: &Block) -> Result<(), StorageError>;
    fn blocks_set_flags(
        &mut self,
        block_id: &BlockId,
        is_committed: Option<bool>,
        is_processed: Option<bool>,
    ) -> Result<(), StorageError>;

    // -------------------------------- BlockDiff -------------------------------- //
    fn block_diffs_insert(&mut self, block_diff: &BlockDiff) -> Result<(), StorageError>;
    fn block_diffs_remove(&mut self, block_id: &BlockId) -> Result<(), StorageError>;

    // -------------------------------- QuorumCertificate -------------------------------- //
    fn quorum_certificates_insert(&mut self, qc: &QuorumCertificate) -> Result<(), StorageError>;

    // -------------------------------- Bookkeeping -------------------------------- //
    fn last_sent_vote_set(&mut self, last_sent_vote: &LastSentVote) -> Result<(), StorageError>;
    fn last_voted_set(&mut self, last_voted: &LastVoted) -> Result<(), StorageError>;
    fn last_votes_unset(&mut self, last_voted: &LastVoted) -> Result<(), StorageError>;
    fn last_executed_set(&mut self, last_exec: &LastExecuted) -> Result<(), StorageError>;
    fn last_proposed_set(&mut self, last_proposed: &LastProposed) -> Result<(), StorageError>;
    fn last_proposed_unset(&mut self, last_proposed: &LastProposed) -> Result<(), StorageError>;
    fn leaf_block_set(&mut self, leaf_node: &LeafBlock) -> Result<(), StorageError>;
    fn locked_block_set(&mut self, locked_block: &LockedBlock) -> Result<(), StorageError>;
    fn high_qc_set(&mut self, high_qc: &HighQc) -> Result<(), StorageError>;
    fn foreign_proposal_upsert(&mut self, foreign_proposal: &ForeignProposal) -> Result<(), StorageError>;
    fn foreign_proposal_delete(&mut self, foreign_proposal: &ForeignProposal) -> Result<(), StorageError>;
    fn foreign_send_counters_set(
        &mut self,
        foreign_send_counter: &ForeignSendCounters,
        block_id: &BlockId,
    ) -> Result<(), StorageError>;
    fn foreign_receive_counters_set(
        &mut self,
        foreign_send_counter: &ForeignReceiveCounters,
    ) -> Result<(), StorageError>;

    // -------------------------------- Transaction -------------------------------- //
    fn transactions_insert(&mut self, transaction: &TransactionRecord) -> Result<(), StorageError>;
    fn transactions_update(&mut self, transaction: &TransactionRecord) -> Result<(), StorageError>;
    fn transactions_save_all<'a, I: IntoIterator<Item = &'a TransactionRecord>>(
        &mut self,
        transaction: I,
    ) -> Result<(), StorageError>;

    fn transactions_finalize_all<'a, I: IntoIterator<Item = &'a TransactionAtom>>(
        &mut self,
        transaction: I,
    ) -> Result<(), StorageError>;
    // -------------------------------- Transaction Executions -------------------------------- //
    fn transaction_executions_insert_or_ignore(
        &mut self,
        transaction_execution: &TransactionExecution,
    ) -> Result<(), StorageError>;

    // -------------------------------- Transaction Pool -------------------------------- //
    fn transaction_pool_insert(
        &mut self,
        transaction: TransactionAtom,
        stage: TransactionPoolStage,
        is_ready: bool,
    ) -> Result<(), StorageError>;
    fn transaction_pool_set_atom(&mut self, transaction: TransactionAtom) -> Result<(), StorageError>;
    fn transaction_pool_add_pending_update(
        &mut self,
        pool_update: &TransactionPoolStatusUpdate,
    ) -> Result<(), StorageError>;

    fn transaction_pool_update(
        &mut self,
        transaction_id: &TransactionId,
        local_decision: Option<Decision>,
        remote_decision: Option<Decision>,
        remote_evidence: Option<&Evidence>,
    ) -> Result<(), StorageError>;
    fn transaction_pool_remove(&mut self, transaction_id: &TransactionId) -> Result<(), StorageError>;
    fn transaction_pool_remove_all<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<Vec<TransactionAtom>, StorageError>;
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
        park_block: &Block,
        missing_transaction_ids: IMissing,
        awaiting_transaction_ids: IAwaiting,
    ) -> Result<(), StorageError>;

    fn missing_transactions_remove(
        &mut self,
        current_height: NodeHeight,
        transaction_id: &TransactionId,
    ) -> Result<Option<Block>, StorageError>;

    // -------------------------------- Votes -------------------------------- //
    fn votes_insert(&mut self, vote: &Vote) -> Result<(), StorageError>;

    //---------------------------------- Substates --------------------------------------------//
    fn substate_locks_insert_all<I: IntoIterator<Item = (SubstateId, Vec<LockedSubstate>)>>(
        &mut self,
        block_id: BlockId,
        locks: I,
    ) -> Result<(), StorageError>;

    fn substate_locks_remove_many_for_transactions<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<(), StorageError>;

    fn substate_down_many<I: IntoIterator<Item = SubstateAddress>>(
        &mut self,
        substate_addresses: I,
        epoch: Epoch,
        destroyed_block_id: &BlockId,
        destroyed_transaction_id: &TransactionId,
        destroyed_qc_id: &QcId,
    ) -> Result<(), StorageError>;
    fn substates_create(&mut self, substate: SubstateRecord) -> Result<(), StorageError>;

    // -------------------------------- Pending State Tree Diffs -------------------------------- //
    fn pending_state_tree_diffs_insert(&mut self, diff: &PendingStateTreeDiff) -> Result<(), StorageError>;
    fn pending_state_tree_diffs_remove_by_block(
        &mut self,
        block_id: &BlockId,
    ) -> Result<PendingStateTreeDiff, StorageError>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum Ordering {
    Ascending,
    Descending,
}
