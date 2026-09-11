//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod shard_scoped_state_tree;
mod substate_value_proof;
use std::{collections::HashMap, ops::Deref};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
pub use shard_scoped_state_tree::*;
pub use substate_value_proof::*;
use tari_consensus_types::{
    BlockId,
    Decision,
    HighPc,
    HighTc,
    HighestSeenBlock,
    LastExecuted,
    LastProposed,
    LastSentNewView,
    LastSentVote,
    LastVoted,
    LeafBlock,
    LockedBlock,
    PcId,
    ProposalCertificate,
    TcId,
    TimeoutCertificate,
};
use tari_engine_types::substate::{Substate, SubstateId};
use tari_ootle_common_types::{
    Epoch,
    NodeAddressable,
    NodeHeight,
    NumPreshards,
    ShardGroup,
    ShardStateVersions,
    SubstateAddress,
    ToSubstateAddress,
    VersionedSubstateIdRef,
    shard::Shard,
};
use tari_ootle_transaction::TransactionId;
use tari_state_tree::{Node, NodeKey, StaleTreeNode, StateTreePayload, Version};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use time::PrimitiveDateTime;

use crate::{
    StorageError,
    consensus_models::{
        Block,
        BlockDiff,
        BlockTransactionExecution,
        EpochCheckpoint,
        Evidence,
        ForeignParkedProposal,
        ForeignProposal,
        ForeignProposalRecord,
        ForeignProposalStatus,
        LockConflict,
        LockedSubstateValue,
        NoVoteReason,
        PendingShardStateTreeDiff,
        StateVersionTransitions,
        SubstateChange,
        SubstateLock,
        SubstatePledges,
        SubstateRecord,
        SubstateUpdateBatch,
        SubstateValueFilterFlags,
        TransactionExecution,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionPoolStatusUpdate,
        TransactionRecord,
        ValidatorConsensusStats,
        ValidatorStatsUpdate,
    },
};

const LOG_TARGET: &str = "tari::ootle::storage";
pub trait StateStore {
    type Addr: NodeAddressable;
    type ReadTransaction<'a>: StateStoreReadTransaction<Addr = Self::Addr>
    where Self: 'a;
    type WriteTransaction<'a>: StateStoreWriteTransaction<Addr = Self::Addr>
        + Deref<Target: StateStoreReadTransaction<Addr = Self::Addr>>
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
    fn current_epoch(&self) -> Result<Epoch, StorageError>;
    fn last_sent_vote_get(&self, epoch: Epoch) -> Result<LastSentVote, StorageError>;
    fn last_voted_get(&self, epoch: Epoch) -> Result<LastVoted, StorageError>;
    fn last_executed_get(&self, epoch: Epoch) -> Result<LastExecuted, StorageError>;
    fn last_proposed_get(&self, epoch: Epoch) -> Result<LastProposed, StorageError>;
    fn locked_block_get(&self, epoch: Epoch) -> Result<LockedBlock, StorageError>;
    fn leaf_block_get(&self, epoch: Epoch) -> Result<LeafBlock, StorageError>;
    /// Returns the stored leaf block regardless of epoch, or `NotFound` if none persisted.
    /// Used by recovery paths that don't know the consensus epoch a priori.
    fn leaf_block_get_any(&self) -> Result<LeafBlock, StorageError>;
    fn highest_seen_block_get(&self, epoch: Epoch) -> Result<HighestSeenBlock, StorageError>;
    fn last_sent_new_view_get(&self, epoch: Epoch) -> Result<LastSentNewView, StorageError>;
    fn high_pc_get(&self, epoch: Epoch) -> Result<HighPc, StorageError>;
    /// Returns the stored high QC pointer regardless of epoch, or `NotFound` if none persisted.
    fn high_pc_get_any(&self) -> Result<HighPc, StorageError>;
    fn high_tc_get(&self, epoch: Epoch) -> Result<HighTc, StorageError>;
    fn is_block_in_end_of_epoch_chain(&self, block_id: &BlockId) -> Result<bool, StorageError>;
    fn foreign_proposals_get_any<'a, I: IntoIterator<Item = &'a BlockId>>(
        &self,
        block_ids: I,
    ) -> Result<Vec<ForeignProposalRecord>, StorageError>;
    fn foreign_proposals_exists(&self, block_id: &BlockId) -> Result<bool, StorageError>;
    fn foreign_proposals_has_unconfirmed(&self, epoch: Epoch) -> Result<bool, StorageError>;
    fn foreign_proposals_get_all_new(
        &self,
        block_id: &BlockId,
        limit: usize,
    ) -> Result<Vec<ForeignProposalRecord>, StorageError>;

    fn transactions_get(&self, tx_id: &TransactionId) -> Result<TransactionRecord, StorageError>;
    fn transactions_exists(&self, tx_id: &TransactionId) -> Result<bool, StorageError>;

    fn transactions_get_any<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &self,
        tx_ids: I,
    ) -> Result<Vec<TransactionRecord>, StorageError>;

    fn finalized_transaction_execution_get(&self, tx_id: &TransactionId) -> Result<TransactionExecution, StorageError>;
    fn finalized_transaction_execution_get_finalized_time(
        &self,
        tx_id: &TransactionId,
    ) -> Result<PrimitiveDateTime, StorageError>;

    fn block_transaction_executions_get_pending_for_block(
        &self,
        tx_id: &TransactionId,
        from_block: &LeafBlock,
    ) -> Result<BlockTransactionExecution, StorageError>;
    fn block_transaction_executions_get_all_for_block(
        &self,
        block_id: &BlockId,
    ) -> Result<Vec<BlockTransactionExecution>, StorageError>;
    fn blocks_get(&self, block_id: &BlockId) -> Result<Block, StorageError>;
    fn blocks_get_all_ids_by_height(&self, epoch: Epoch, height: NodeHeight) -> Result<Vec<BlockId>, StorageError>;
    fn blocks_get_genesis_for_epoch(&self, epoch: Epoch) -> Result<Block, StorageError>;
    /// Returns all blocks from and excluding the start block (lower height) to the end block (inclusive)
    fn blocks_get_all_between(
        &self,
        epoch: Epoch,
        start_block_height: NodeHeight,
        end_block_height: NodeHeight,
        include_dummy_blocks: bool,
        limit: usize,
    ) -> Result<Vec<Block>, StorageError>;
    fn blocks_exists(&self, block_id: &BlockId) -> Result<bool, StorageError>;
    fn blocks_is_pending_ancestor(&self, descendant: &BlockId, ancestor: &BlockId) -> Result<bool, StorageError>;
    fn blocks_get_committed_by_parent(&self, parent: &BlockId) -> Result<Block, StorageError>;
    fn blocks_get_pending_ids_by_parent(&self, parent: &BlockId) -> Result<Vec<BlockId>, StorageError>;

    fn blocks_get_paginated(
        &self,
        limit: u64,
        offset: u64,
        filter_index: Option<usize>,
        filter: Option<String>,
        ordering_index: Option<usize>,
        ordering: Option<Ordering>,
    ) -> Result<Vec<Block>, StorageError>;

    fn filtered_blocks_get_count(
        &self,
        filter_index: Option<usize>,
        filter: Option<String>,
    ) -> Result<u64, StorageError>;

    fn block_diffs_get(&self, block_id: &BlockId) -> Result<BlockDiff, StorageError>;

    /// Returns the last change to `substate_id` recorded by the branch ending at `block_id`, or `NotFound` if the
    /// branch contains no change for it.
    ///
    /// Implementations must only consider changes recorded by blocks in that branch: a change from a forked-out
    /// sibling or any other subtree must never be returned. A substate version is only ever DOWNed after it is UPed,
    /// so where a branch both UPs and DOWNs the same version, the DOWN is the last change.
    fn block_diffs_get_last_change_for_substate(
        &self,
        block_id: &BlockId,
        substate_id: &SubstateId,
    ) -> Result<SubstateChange, StorageError>;
    /// Returns the last change to the given substate version recorded by the branch ending at `block_id`, or
    /// `NotFound` if the branch contains no change for it. Selection follows the same rules as
    /// [`Self::block_diffs_get_last_change_for_substate`], restricted to the requested version.
    fn block_diffs_get_change_for_versioned_substate<'a, T: Into<VersionedSubstateIdRef<'a>>>(
        &self,
        block_id: &BlockId,
        substate_id: T,
    ) -> Result<SubstateChange, StorageError>;

    // -------------------------------- ProposalCertificate -------------------------------- //
    fn proposal_certificates_get(&self, epoch: Epoch, qc_id: &PcId) -> Result<ProposalCertificate, StorageError>;
    fn proposal_certificates_get_many<'a, I>(&self, qc_ids: I) -> Result<Vec<ProposalCertificate>, StorageError>
    where
        I: IntoIterator<Item = &'a (Epoch, PcId)>,
        I::IntoIter: ExactSizeIterator;

    // -------------------------------- TimeoutCertificate -------------------------------- //
    fn timeout_certificates_get(&self, epoch: Epoch, id: &TcId) -> Result<TimeoutCertificate, StorageError>;
    fn timeout_certificates_get_many<'a, I>(&self, ids: I) -> Result<Vec<TimeoutCertificate>, StorageError>
    where
        I: IntoIterator<Item = &'a (Epoch, TcId)>,
        I::IntoIter: ExactSizeIterator;

    // -------------------------------- Transaction Pools -------------------------------- //
    fn transaction_pool_get_for_blocks(
        &self,
        to_block_id: &BlockId,
        transaction_id: &TransactionId,
    ) -> Result<TransactionPoolRecord, StorageError>;
    fn transaction_pool_exists(&self, transaction_id: &TransactionId) -> Result<bool, StorageError>;
    fn transaction_pool_get_all(&self, limit: usize) -> Result<Vec<TransactionPoolRecord>, StorageError>;
    fn transaction_pool_get_many_ready(
        &self,
        weight_budget: u64,
        max_count: usize,
        block_id: &BlockId,
    ) -> Result<Vec<TransactionPoolRecord>, StorageError>;
    fn transaction_pool_has_pending_state_updates(&self, block_id: &BlockId) -> Result<bool, StorageError>;

    // TODO: just check for existence
    fn transaction_pool_count(
        &self,
        stage: Option<TransactionPoolStage>,
        is_ready: Option<bool>,
        skip_lock_conflicted: bool,
    ) -> Result<usize, StorageError>;

    //---------------------------------- Substates --------------------------------------------//
    fn substates_get(&self, address: &SubstateAddress) -> Result<SubstateRecord, StorageError>;
    fn substates_get_any<'a, I: IntoIterator<Item = &'a VersionedSubstateIdRef<'a>>>(
        &self,
        substate_ids: I,
    ) -> Result<Vec<SubstateRecord>, StorageError>;
    fn substates_get_any_max_version<'a, I>(&self, substate_ids: I) -> Result<Vec<SubstateRecord>, StorageError>
    where
        I: IntoIterator<Item = &'a SubstateId>,
        I::IntoIter: ExactSizeIterator;
    /// Returns (version, is_up)
    fn substates_get_max_version_for_substate(&self, substate_id: &SubstateId) -> Result<(u64, bool), StorageError>;
    fn substates_any_exist<'a, I>(&self, substates: I) -> Result<bool, StorageError>
    where I: IntoIterator<Item = VersionedSubstateIdRef<'a>>;

    /// Returns true if any version of the given substate id exists, up or down. A pure key-existence
    /// check — cheaper than fetching the head version when the caller does not need it.
    fn substates_exists_any_version(&self, substate_id: &SubstateId) -> Result<bool, StorageError>;

    /// Returns an iterator of all substates in the given shard, starting from the given state version (inclusive).
    /// It returns substates, ordered by state version however, the ordering within state versions is by the node key,
    /// meaning it is essentially random. WARNING: do not use this as it only works when all stale state tree nodes
    /// have been pruned. TODO: the JMT crate provides an iterator that incorporates JMT logic. This iterator is
    /// more performant because it iterates in natural key order but returns duplicate results unless stale nodes
    /// are fully pruned. We should consider implementing a correct iterator that incorporates JMT logic to avoid this
    /// issue.
    fn substate_tree_iter(
        &self,
        shard: Shard,
        starting_from_state_version: Version,
        value_filters: SubstateValueFilterFlags,
    ) -> Result<impl Iterator<Item = Result<(Version, SubstateId, Substate), StorageError>>, StorageError>;

    // -------------------------------- SubstateLocks -------------------------------- //

    fn substate_locks_get_locked_substates_for_transaction(
        &self,
        transaction_id: &TransactionId,
    ) -> Result<Vec<LockedSubstateValue>, StorageError>;

    fn substate_locks_has_any_write_locks_for_substates<'a, I: IntoIterator<Item = &'a SubstateId>>(
        &self,
        exclude_transaction_id: Option<&TransactionId>,
        substate_ids: I,
    ) -> Result<Option<TransactionId>, StorageError>;

    fn substate_locks_get_latest_for_substate(
        &self,
        leaf_block: &LeafBlock,
        substate_id: &SubstateId,
    ) -> Result<SubstateLock, StorageError>;

    fn pending_state_tree_diffs_get_all_up_to_commit_block(
        &self,
        block_id: &BlockId,
    ) -> Result<HashMap<Shard, Vec<PendingShardStateTreeDiff>>, StorageError>;

    // -------------------------------- State transitions -------------------------------- //
    fn state_transitions_get_starting_at(
        &self,
        shard: Shard,
        state_version: Version,
        value_filters: SubstateValueFilterFlags,
    ) -> Result<StateVersionTransitions, StorageError>;

    // -------------------------------- State Tree -------------------------------- //

    fn state_tree_nodes_get(&self, shard: Shard, key: &NodeKey) -> Result<Node<StateTreePayload>, StorageError>;
    fn state_tree_nodes_get_all_by_state_version(
        &self,
        shard: Shard,
        state_version: Version,
    ) -> Result<Vec<(NodeKey, Node<StateTreePayload>)>, StorageError>;
    fn state_tree_versions_get_latest(&self, shard: Shard) -> Result<Option<Version>, StorageError>;
    fn state_tree_versions_get_latest_for_shard_group(
        &self,
        shard_group: ShardGroup,
    ) -> Result<ShardStateVersions, StorageError>;

    // -------------------------------- Epoch checkpoint -------------------------------- //
    fn epoch_checkpoint_get_all_from_epoch(
        &self,
        epoch: Epoch,
        limit: usize,
    ) -> Result<Vec<EpochCheckpoint>, StorageError>;
    fn epoch_checkpoint_get_by_shard_group(
        &self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<EpochCheckpoint, StorageError>;

    fn epoch_checkpoint_get_last(&self) -> Result<EpochCheckpoint, StorageError>;

    // -------------------------------- Foreign Substate Pledges -------------------------------- //
    fn foreign_substate_pledges_exists_for_transaction_and_address<T: ToSubstateAddress>(
        &self,
        transaction_id: &TransactionId,
        address: T,
    ) -> Result<bool, StorageError>;
    fn foreign_substate_pledges_get_write_pledges_to_transaction<'a, I: IntoIterator<Item = &'a SubstateId>>(
        &self,
        transaction_id: &TransactionId,
        substate_ids: I,
    ) -> Result<SubstatePledges, StorageError>;
    fn foreign_substate_pledges_get_all_by_transaction_id(
        &self,
        transaction_id: &TransactionId,
    ) -> Result<SubstatePledges, StorageError>;
    // -------------------------------- Parked blocks / Missing Transactions -------------------------------- //
    fn parked_block_exists(&self, block_id: &BlockId) -> Result<bool, StorageError>;

    // -------------------------------- Foreign parked block -------------------------------- //
    fn foreign_parked_blocks_exists(&self, block_id: &BlockId) -> Result<bool, StorageError>;

    // -------------------------------- ValidatorNodeStats -------------------------------- //
    fn validator_epoch_stats_get(
        &self,
        epoch: Epoch,
        public_key: &RistrettoPublicKeyBytes,
    ) -> Result<ValidatorConsensusStats, StorageError>;
}

pub trait StateStoreWriteTransaction {
    type Addr: NodeAddressable;

    fn commit(&mut self) -> Result<(), StorageError>;
    fn rollback(&mut self) -> Result<(), StorageError>;

    // -------------------------------- Block -------------------------------- //
    fn blocks_insert(&mut self, block: &Block) -> Result<(), StorageError>;
    fn blocks_delete(&mut self, block_id: &BlockId) -> Result<(), StorageError>;
    fn blocks_set_qcs(
        &mut self,
        block_id: &BlockId,
        commit_qc_id: Option<&PcId>,
        justify_qc_id: Option<&PcId>,
    ) -> Result<(), StorageError>;

    // -------------------------------- BlockDiff -------------------------------- //
    fn block_diffs_insert(&mut self, block_id: &BlockId, changes: &[SubstateChange]) -> Result<(), StorageError>;
    fn block_diffs_remove(&mut self, block_id: &BlockId) -> Result<(), StorageError>;

    // -------------------------------- ProposalCertificate -------------------------------- //
    fn proposal_certificates_save(&mut self, qc: &ProposalCertificate) -> Result<(), StorageError>;
    // -------------------------------- TimeoutCertificate -------------------------------- //
    fn timeout_certificates_save(&mut self, tc: &TimeoutCertificate) -> Result<(), StorageError>;

    // -------------------------------- Bookkeeping -------------------------------- //
    fn last_sent_vote_set(&mut self, last_sent_vote: &LastSentVote) -> Result<(), StorageError>;
    fn last_voted_set(&mut self, last_voted: &LastVoted) -> Result<(), StorageError>;
    fn last_executed_set(&mut self, last_exec: &LastExecuted) -> Result<(), StorageError>;
    fn last_proposed_set(&mut self, last_proposed: &LastProposed) -> Result<(), StorageError>;
    fn leaf_block_set(&mut self, leaf_node: &LeafBlock) -> Result<(), StorageError>;
    fn highest_seen_block_set(&mut self, last_seen_block: &HighestSeenBlock) -> Result<(), StorageError>;
    fn last_sent_new_view_set(&mut self, last_sent_new_view: &LastSentNewView) -> Result<(), StorageError>;
    /// Clears the last sent new view record for the current epoch. If there is no record, it is a no-op.
    fn last_sent_new_view_clear(&mut self) -> Result<(), StorageError>;
    fn locked_block_set(&mut self, locked_block: &LockedBlock) -> Result<(), StorageError>;
    fn high_pc_set(&mut self, high_pc: &HighPc) -> Result<(), StorageError>;
    fn high_tc_set(&mut self, high_tc: &HighTc) -> Result<(), StorageError>;
    fn foreign_proposals_save(&mut self, foreign_proposal: &ForeignProposalRecord) -> Result<(), StorageError>;
    fn foreign_proposals_delete(&mut self, block_id: &BlockId) -> Result<(), StorageError>;

    fn foreign_proposals_set_status(
        &mut self,
        block_id: &BlockId,
        status: ForeignProposalStatus,
        set_proposed_in_block: Option<&BlockId>,
    ) -> Result<(), StorageError>;

    fn foreign_proposals_clear_proposed_in(&mut self, proposed_in_block: &BlockId) -> Result<(), StorageError>;

    // -------------------------------- Transaction -------------------------------- //
    fn transactions_insert(&mut self, transaction: &TransactionRecord) -> Result<(), StorageError>;

    /// Marks the given transactions as finalized in the epoch of the finalizing block. The epoch is
    /// recorded so that epoch GC can prune finalized transaction bookkeeping once it ages out of the
    /// retained window.
    fn transactions_finalize_all<'a, I: IntoIterator<Item = &'a TransactionPoolRecord>>(
        &mut self,
        epoch: Epoch,
        transactions: I,
    ) -> Result<(), StorageError>;

    /// Forgets the node-local finalized bookkeeping for a transaction: the finalized marker and all
    /// recorded executions. This never reverts committed state — a commit's effects, fees and
    /// receipt substate live in synced state — it only makes the id appear unsequenced to this
    /// node's records. Used to give an aborted transaction a fresh lifecycle, and by record GC to
    /// prune old bookkeeping; committed ids remain refused via the receipt-existence check, which
    /// does not depend on these records.
    fn transactions_finalized_remove(&mut self, tx_id: &TransactionId) -> Result<(), StorageError>;

    // -------------------------------- Transaction Executions -------------------------------- //

    fn block_transaction_executions_insert_or_ignore(
        &mut self,
        transaction_execution: &BlockTransactionExecution,
    ) -> Result<bool, StorageError>;

    fn block_transaction_executions_remove_any_by_block_id(&mut self, block_id: &BlockId) -> Result<(), StorageError>;
    fn block_transaction_executions_lock_any_for_block(&mut self, block: &LeafBlock) -> Result<(), StorageError>;

    // -------------------------------- Transaction Pool -------------------------------- //
    fn transaction_pool_insert_new(
        &mut self,
        tx_id: TransactionId,
        decision: Decision,
        initial_evidence: &Evidence,
        is_ready: bool,
        is_global: bool,
        max_epoch: Epoch,
        transaction_weight: u64,
    ) -> Result<(), StorageError>;
    fn transaction_pool_add_pending_update(
        &mut self,
        block: &LeafBlock,
        pool_update: &TransactionPoolStatusUpdate,
    ) -> Result<(), StorageError>;

    fn transaction_pool_remove_all<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<Vec<TransactionPoolRecord>, StorageError>;
    fn transaction_pool_confirm_all_transitions(&mut self, block: &LeafBlock) -> Result<(), StorageError>;
    fn transaction_pool_state_updates_remove_any_by_block_id(&mut self, block_id: &BlockId)
    -> Result<(), StorageError>;

    // -------------------------------- Parked blocks / Missing Transactions -------------------------------- //

    fn parked_block_insert<'a, IMissing: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        park_block: &Block,
        foreign_proposals: &[ForeignProposal],
        missing_transaction_ids: IMissing,
    ) -> Result<(), StorageError>;

    fn parked_block_remove_missing_transaction(
        &mut self,
        height: NodeHeight,
        transaction_id: &TransactionId,
    ) -> Result<Option<(Block, Vec<ForeignProposal>)>, StorageError>;

    // -------------------------------- Foreign parked block -------------------------------- //
    fn foreign_parked_blocks_insert(&mut self, park_block: &ForeignParkedProposal) -> Result<(), StorageError>;

    fn foreign_parked_blocks_insert_missing_transactions<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        park_block_id: &BlockId,
        missing_transaction_ids: I,
    ) -> Result<(), StorageError>;

    fn foreign_parked_blocks_remove_all_by_transaction(
        &mut self,
        transaction_id: &TransactionId,
    ) -> Result<Vec<ForeignParkedProposal>, StorageError>;

    //---------------------------------- Substates --------------------------------------------//
    fn substate_locks_insert_all<'a, I: IntoIterator<Item = (&'a SubstateId, &'a Vec<SubstateLock>)>>(
        &mut self,
        block: &LeafBlock,
        locks: I,
    ) -> Result<(), StorageError>;

    fn substate_locks_remove_many_for_transactions<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<(), StorageError>;

    fn substate_locks_remove_any_by_block_id(&mut self, block_id: &BlockId) -> Result<(), StorageError>;

    // -------------------------------- Substates -------------------------------- //

    fn substates_commit_batch(&mut self, update_batch: SubstateUpdateBatch) -> Result<(), StorageError>;
    fn substates_prune_downed_values(&mut self, epoch: Epoch) -> Result<usize, StorageError>;

    // -------------------------------- Foreign pledges -------------------------------- //

    fn foreign_substate_pledges_save(
        &mut self,
        transaction_id: &TransactionId,
        shard_group: ShardGroup,
        pledges: &SubstatePledges,
    ) -> Result<(), StorageError>;

    fn foreign_substate_pledges_remove_many<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<(), StorageError>;

    // -------------------------------- Pending State Tree Diffs -------------------------------- //
    fn pending_state_tree_diffs_insert(
        &mut self,
        block_id: BlockId,
        shard: Shard,
        diff: &PendingShardStateTreeDiff,
    ) -> Result<(), StorageError>;
    fn pending_state_tree_diffs_remove_by_block(&mut self, block_id: &BlockId) -> Result<(), StorageError>;
    fn pending_state_tree_diffs_remove_and_return_by_block(
        &mut self,
        block_id: &BlockId,
    ) -> Result<IndexMap<Shard, Vec<PendingShardStateTreeDiff>>, StorageError>;

    //---------------------------------- State tree --------------------------------------------//
    fn state_tree_nodes_batch_insert(
        &mut self,
        shard: Shard,
        nodes: Vec<(NodeKey, Node<StateTreePayload>)>,
    ) -> Result<(), StorageError>;

    fn state_tree_nodes_record_stale_tree_nodes(
        &mut self,
        shard: Shard,
        version: Version,
        nodes: Vec<StaleTreeNode>,
    ) -> Result<(), StorageError>;

    fn state_tree_nodes_clear_stale(&mut self, num_preshards: NumPreshards) -> Result<usize, StorageError>;
    fn state_tree_shard_versions_set(&mut self, shard: Shard, version: Version) -> Result<(), StorageError>;

    // -------------------------------- Epoch checkpoint -------------------------------- //
    fn epoch_checkpoint_save(&mut self, checkpoint: &EpochCheckpoint) -> Result<(), StorageError>;

    // -------------------------------- Lock conflicts -------------------------------- //
    fn lock_conflicts_insert_all<'a, I: IntoIterator<Item = (&'a TransactionId, &'a Vec<LockConflict>)>>(
        &mut self,
        block_id: &BlockId,
        conflicts: I,
    ) -> Result<(), StorageError>;

    fn lock_conflicts_remove_by_transaction_ids<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<(), StorageError>;

    fn lock_conflicts_remove_by_block_id(&mut self, block_id: &BlockId) -> Result<(), StorageError>;

    // -------------------------------- ParticipationShares -------------------------------- //
    fn validator_epoch_stats_updates<'a, I: IntoIterator<Item = ValidatorStatsUpdate<'a>>>(
        &mut self,
        epoch: Epoch,
        updates: I,
    ) -> Result<(), StorageError>;

    // -------------------------------- Epoch cleanup -------------------------------- //
    fn epoch_cleanup(&mut self, epoch: Epoch) -> Result<(), StorageError>;

    // -------------------------------- Diagnotics -------------------------------- //
    fn diagnostics_add_no_vote(&mut self, block_id: BlockId, reason: NoVoteReason) -> Result<(), StorageError>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Ordering {
    // Use default only where you "don't care" about the order. Ascending is more performant so it's the default.
    #[default]
    Ascending,
    Descending,
}

impl Ordering {
    pub fn is_ascending(&self) -> bool {
        matches!(self, Self::Ascending)
    }

    pub fn is_descending(&self) -> bool {
        matches!(self, Self::Descending)
    }
}
