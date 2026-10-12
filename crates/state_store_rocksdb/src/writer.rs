//  Copyright 2024. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

// A write transaction must not be written through at the key its iterator is standing on. The eager accessors
// (query_prefix_range_keys and friends) are the way to read a range here.
#![deny(clippy::disallowed_methods)]

use std::{collections::HashSet, iter, ops::Deref, sync::Arc};

use indexmap::IndexMap;
use log::*;
use rocksdb::{Transaction, TransactionDB};
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
    TimeoutCertificate,
};
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{
    Epoch,
    NodeAddressable,
    NodeHeight,
    NumPreshards,
    ShardGroup,
    ToSubstateAddress,
    VersionedSubstateId,
    optional::Optional,
    shard::Shard,
};
use tari_ootle_storage::{
    EpochCleanupStep,
    Ordering,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StateTreeTruncateStats,
    StorageError,
    SubstateRewindStats,
    consensus_models::{
        Block,
        BlockTransactionExecution,
        EpochCheckpoint,
        Evidence,
        ForeignParkedProposal,
        ForeignProposal,
        ForeignProposalRecord,
        ForeignProposalStatus,
        LockConflict,
        NoVoteReason,
        PendingShardStateTreeDiff,
        StateVersionProof,
        SubstateChange,
        SubstateCreated,
        SubstateDestroyed,
        SubstateDownProofRecord,
        SubstateLock,
        SubstatePledges,
        SubstateRecord,
        SubstateTransition,
        SubstateUpdateBatch,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionPoolStatusUpdate,
        TransactionRecord,
        ValidatorStatsUpdate,
        VoteEquivocation,
    },
    time,
};
use tari_ootle_transaction::TransactionId;
use tari_state_tree::{Children, Node, NodeKey, NodeType, StaleTreeNode, StateTreePayload, Version};

use crate::{
    block_diff_table::BlockDiffEntry,
    cf_api::{CfContext, DbContext},
    codecs::{ByteColumn, DbEncoder, DefaultCodec, KeyPrefix, StaleTreeNodesCodec},
    column_families::{
        block,
        block::BlockCf,
        block_diff::BlockDiffRecordCf,
        block_transaction_execution,
        block_transaction_execution::BlockTransactionExecutionCf,
        bookkeeping::{
            CommitBlock,
            CommitBlockCf,
            HighPcCf,
            HighTcCf,
            HighestSeenBlockCf,
            LastCommittedEpochEndCf,
            LastExecutedCf,
            LastProposedCf,
            LastSentNewViewCf,
            LastSentVoteCf,
            LastVotedCf,
            LeafBlockCf,
            LockedBlockCf,
        },
        certificates::{proposal::ProposalCertificateCf, timeout::TimeoutCertificateCf},
        chain,
        chain::PendingChainIndex,
        diagnostic_no_vote::{DiagnosticsNoVoteCf, DiagnosticsNoVoteData},
        epoch_checkpoint::EpochCheckpointCf,
        finalized_transaction,
        finalized_transaction::{FinalizedTransactionLinkCf, FinalizedTransactionLinkData},
        foreign_parked_blocks,
        foreign_parked_blocks::ForeignParkedBlockCf,
        foreign_proposal,
        foreign_proposal::{ForeignProposalCf, ForeignProposalEpochIndexData},
        foreign_substate_pledge,
        foreign_substate_pledge::ForeignSubstatePledgeCf,
        lock_conflict,
        lock_conflict::LockConflictCf,
        missing_transactions,
        missing_transactions::MissingTransactionCf,
        parked_block::{ParkedBlockCf, ParkedBlockDataRef},
        pending_state_tree_diff::{PendingStateTreeDiffRecordCf, ShardStateTreeDiff, ShardStateTreeDiffRef},
        state_sync_rewind_point::StateSyncRewindPointCf,
        state_transition,
        state_transition::{
            StateTransitionCf,
            StateTransitionModelDataV1,
            StateTransitionRecordData,
            StateTransitionType,
        },
        state_tree,
        state_tree::{StateTreeCf, StateTreeStaleNodesCf},
        state_tree_shard_versions::StateTreeShardVersionCf,
        state_version_proof::{BlockCommitProofCf, StateVersionProofCf},
        substate,
        substate::{SubstateCf, SubstateHeadData},
        substate_down_proof::SubstateDownProofCf,
        substate_locks::BlockLockSetCf,
        transaction::TransactionCf,
        transaction_pool::TransactionPoolCf,
        transaction_pool_state_update,
        transaction_pool_state_update::{TransactionPoolStateUpdateCf, TransactionPoolStateUpdateData},
        validator_liveness_log::{self, ValidatorLivenessLogCf},
        validator_node_epoch_stats::ValidatorNodeEpochStatsCf,
        vote_equivocation,
    },
    error::RocksDbStorageError,
    lock_table::BlockLocks,
    options::DatabaseOptions,
    pending_state::{PendingStateView, SharedPendingState, StagedPendingState},
    read_only::ReadOnly,
    reader::RocksDbStateStoreReadTransaction,
    utils::now,
};

const LOG_TARGET: &str = "tari::ootle::storage::state_store_rocksdb::writer";

pub type DbWriteContext<'a> = DbContext<'a, Transaction<'a, TransactionDB>>;

pub struct RocksDbStateStoreWriteTransaction<'a, TAddr> {
    /// None indicates if the transaction has been explicitly committed/rolled back
    transaction: Option<RocksDbStateStoreReadTransaction<'a, TAddr>>,
    db: &'a TransactionDB,
    options: &'a DatabaseOptions,
}

impl<'a, TAddr: NodeAddressable> RocksDbStateStoreWriteTransaction<'a, TAddr> {
    pub(crate) fn new(
        db: &'a TransactionDB,
        tx: Transaction<'a, TransactionDB>,
        pending: &'a SharedPendingState,
        options: &'a DatabaseOptions,
    ) -> Self {
        Self {
            db,
            // We have access to the inner transaction so we can use it to read/write
            transaction: Some(RocksDbStateStoreReadTransaction::new(
                db,
                ReadOnly::new(tx),
                PendingStateView::writer(pending),
            )),
            options,
        }
    }

    pub fn db(&self) -> DbWriteContext<'_> {
        DbContext::new(self.db, self.tx())
    }

    fn tx(&self) -> &Transaction<'_, TransactionDB> {
        self.transaction
            .as_ref()
            .expect("DB transaction already taken")
            .rocksdb_transaction()
    }

    fn staged(&mut self) -> &mut StagedPendingState {
        self.transaction
            .as_mut()
            .expect("DB transaction already taken")
            .pending_mut()
            .staged_mut()
    }

    /// Writes `block_id`'s lock-set record to match the staged table: rewritten while the block holds locks, deleted
    /// once it holds none.
    fn write_block_lock_set(&mut self, block_id: &BlockId, operation: &'static str) -> Result<(), StorageError> {
        let record = self
            .staged()
            .state()
            .locks
            .block(block_id)
            .map(|locks| locks.to_record());
        let cf = self.db().cf(BlockLockSetCf)?;
        match record {
            Some(record) => cf.put(block_id, &record, operation)?,
            None => cf.delete(block_id, operation)?,
        }
        Ok(())
    }

    /// [`StateStoreWriteTransaction::substate_locks_insert_all`] for the block at `block_epoch` and `block_height`.
    pub fn substate_locks_insert_for_block<'b, I: IntoIterator<Item = (&'b SubstateId, &'b Vec<SubstateLock>)>>(
        &mut self,
        block_id: &BlockId,
        block_epoch: Epoch,
        block_height: NodeHeight,
        locks: I,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "substate_locks_insert_for_block";

        let mut substates = IndexMap::<SubstateId, Vec<SubstateLock>>::new();
        for (substate_id, locks) in locks {
            if !locks.is_empty() {
                substates.entry(substate_id.clone()).or_default().extend(locks);
            }
        }
        if substates.is_empty() {
            return self.remove_block_lock_set(block_id, OPERATION);
        }

        let locks = BlockLocks {
            epoch: block_epoch,
            height: block_height,
            substates,
        };
        self.db()
            .cf(BlockLockSetCf)?
            .put(block_id, &locks.to_record(), OPERATION)?;
        self.staged().locks_mut().insert_block(*block_id, locks);

        Ok(())
    }

    fn remove_block_lock_set(&mut self, block_id: &BlockId, operation: &'static str) -> Result<(), StorageError> {
        if self.pending_state().locks.block(block_id).is_none() {
            return Ok(());
        }
        self.staged().locks_mut().remove_block(block_id);
        self.db().cf(BlockLockSetCf)?.delete(block_id, operation)?;
        Ok(())
    }

    fn parked_blocks_insert(
        &mut self,
        block: &Block,
        foreign_proposals: &[ForeignProposal],
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "parked_blocks_insert";
        if self.blocks_exists(block.id())? {
            return Err(StorageError::QueryError {
                reason: format!(
                    "Cannot park block {} that already exists in the blocks table",
                    block.id()
                ),
            });
        }

        let cf = self.db().cf(ParkedBlockCf)?;
        // Idempotent
        if cf.exists(block.id(), OPERATION)? {
            return Ok(());
        }

        let codec = DefaultCodec::<ParkedBlockDataRef<'_>>::default();
        // We need to clone these because the current design does not allow "encode only" CFs/Codecs
        // which would not have the 'static lifetime requirement on the value
        let parked_block_data = ParkedBlockDataRef {
            block,
            foreign_proposals,
        };
        let data = codec.encode(&parked_block_data)?;
        cf.put_raw_value(block.id(), &data, OPERATION)?;

        Ok(())
    }

    fn parked_blocks_remove(&mut self, block_id: &BlockId) -> Result<(Block, Vec<ForeignProposal>), StorageError> {
        const OPERATION: &str = "parked_blocks_remove";
        let cf = self.db().cf(ParkedBlockCf)?;
        let data = cf.get(block_id, OPERATION)?;
        cf.delete_or_not_found(block_id, OPERATION)?;

        Ok((data.block, data.foreign_proposals))
    }
}

impl<'tx, TAddr: NodeAddressable + 'tx> StateStoreWriteTransaction for RocksDbStateStoreWriteTransaction<'tx, TAddr> {
    type Addr = TAddr;

    fn commit(&mut self) -> Result<(), StorageError> {
        // Take so that we mark this transaction as complete in the drop impl
        let tx = self.transaction.take().expect("commit: already committed");

        let (tx, locks) = tx.into_parts();
        let commit = || {
            tx.commit().map_err(|source| RocksDbStorageError::RocksDbError {
                source,
                operation: "commit",
            })
        };
        match locks.into_staged() {
            Some((shared, staged)) => shared.commit_and_publish(staged, commit)?,
            None => commit()?,
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), StorageError> {
        // Take so that we mark this transaction as complete in the drop impl
        self.transaction
            .take()
            .expect("rollback: already committed")
            .into_parts()
            .0
            .rollback()
            .map_err(|source| RocksDbStorageError::RocksDbError {
                source,
                operation: "commit",
            })?;
        Ok(())
    }

    fn blocks_insert(&mut self, block: &Block) -> Result<(), StorageError> {
        const OPERATION: &str = "blocks_insert";
        let cf = self.db().cf(BlockCf)?;
        if cf.exists(block.id(), OPERATION)? {
            return Err(StorageError::QueryError {
                reason: format!("Block {} already exists", block.id()),
            });
        }
        // TODO: we're storing the QC twice.
        cf.put(block.id(), block, OPERATION)?;

        let index_cf = self.db().cf(block::EpochHeightIndex)?;
        index_cf.put(&(block.epoch(), block.height(), *block.id()), &(), OPERATION)?;

        if !block.id().is_zero() {
            let chain_cf = self.db().cf(PendingChainIndex)?;
            chain_cf.put(block.id(), block.parent(), OPERATION)?;
            let parent_child_cf = self.db().cf(chain::PendingParentChildIndex)?;
            parent_child_cf.put(&(*block.parent(), *block.id()), &(), OPERATION)?;
        }

        // TODO: the SQLite implementation updates the block time from the last block. Ideally we remove the need for
        // this (JRPC server/client can just determine it themselves?)
        //
        // let maybe_last = cf.get_last(OPERATION).optional()?; let next_block_time = match maybe_last {
        //     Some((_, last)) => last.block_time().map(|t| block.timestamp().saturating_sub(t) ),
        //     None => {
        //         SystemTime::now()
        //             .duration_since(UNIX_EPOCH)
        //             .map_err(|e| StorageError::General { details: e.to_string() })?
        //             .as_millis()
        //             .try_into()
        //             .unwrap()
        //     },
        // };
        //
        // block.set_block_time(next_block_time);

        Ok(())
    }

    fn blocks_delete(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        const OPERATION: &str = "blocks_delete";
        let cf = self.db().cf(BlockCf)?;
        // Let's be a little paranoid and check this call is valid since it is destructive
        let block = cf.get(block_id, OPERATION)?;
        if block.is_committed() {
            return Err(StorageError::QueryError {
                reason: format!("Cannot delete committed block {}", block_id),
            });
        }
        cf.delete(block_id, OPERATION)?;

        let index_cf = self.db().cf(block::EpochHeightIndex)?;
        index_cf.delete(&(block.epoch(), block.height(), *block.id()), OPERATION)?;

        // TODO: could lead to orphan chains left in DB - need to recursively remove all children
        let chain_cf = self.db().cf(PendingChainIndex)?;
        chain_cf.delete(block_id, OPERATION)?;
        let parent_child_cf = self.db().cf(chain::PendingParentChildIndex)?;
        parent_child_cf.delete(&(*block.parent(), *block.id()), OPERATION)?;

        Ok(())
    }

    fn blocks_set_qcs(
        &mut self,
        block_id: &BlockId,
        commit_qc_id: Option<&PcId>,
        justify_qc_id: Option<&PcId>,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "blocks_set_qcs";
        if commit_qc_id.is_none() && justify_qc_id.is_none() {
            return Ok(());
        }

        let cf = self.db().cf(BlockCf)?;
        let mut block = cf.get(block_id, OPERATION)?;

        // set the flags
        if let Some(qc_id) = commit_qc_id {
            block.set_commit_qc(*qc_id);
            // The block is committed, remove it from the pending chain
            self.db().cf(PendingChainIndex)?.delete(block_id, OPERATION)?;
            self.db()
                .cf(chain::PendingParentChildIndex)?
                .delete(&(*block.parent(), *block_id), OPERATION)?;
            self.db()
                .cf(chain::CommittedParentChildChainIndex)?
                .put(block.parent(), block_id, OPERATION)?;
            self.db().cf(CommitBlockCf)?.put(
                &ByteColumn,
                &CommitBlock {
                    epoch: block.epoch(),
                    height: block.height(),
                    block_id: *block.id(),
                    parent_id: *block.parent(),
                },
                OPERATION,
            )?;
            if block.is_epoch_end() {
                self.db()
                    .cf(LastCommittedEpochEndCf)?
                    .put(&ByteColumn, &block.as_leaf(), OPERATION)?;
            }
        }
        if let Some(value) = justify_qc_id {
            block.set_justify_qc(*value);
        }

        cf.put(block_id, &block, OPERATION)?;

        Ok(())
    }

    fn block_diffs_insert(&mut self, block_id: &BlockId, changes: &[SubstateChange]) -> Result<(), StorageError> {
        const OPERATION: &str = "block_diffs_insert";
        if changes.is_empty() {
            return self.block_diffs_remove(block_id);
        }

        let encoded = tari_bor::encode(&changes).map_err(|e| RocksDbStorageError::EncodeError { source: e.into() })?;
        self.db()
            .cf(BlockDiffRecordCf)?
            .put_raw_value(block_id, &encoded, OPERATION)?;
        self.staged()
            .block_diffs_mut()
            .insert(*block_id, Arc::new(BlockDiffEntry::new(changes.to_vec())));

        Ok(())
    }

    fn block_diffs_remove(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        const OPERATION: &str = "block_diffs_remove";
        if self.pending_state().block_diffs.get(block_id).is_none() {
            return Ok(());
        }
        self.staged().block_diffs_mut().remove(block_id);
        self.db().cf(BlockDiffRecordCf)?.delete(block_id, OPERATION)?;

        Ok(())
    }

    fn proposal_certificates_save(&mut self, qc: &ProposalCertificate) -> Result<(), StorageError> {
        const OPERATION: &str = "proposal_certificates_save";
        self.db()
            .cf(ProposalCertificateCf)?
            .put(&(qc.epoch(), qc.calculate_id()), qc, OPERATION)?;
        Ok(())
    }

    fn timeout_certificates_save(&mut self, tc: &TimeoutCertificate) -> Result<(), StorageError> {
        const OPERATION: &str = "timeout_certificates_save";

        self.db()
            .cf(TimeoutCertificateCf)?
            .put(&(tc.epoch(), tc.calculate_id()), tc, OPERATION)?;
        Ok(())
    }

    fn last_sent_vote_set(&mut self, last_sent_vote: &LastSentVote) -> Result<(), StorageError> {
        self.db()
            .cf(LastSentVoteCf)?
            .put(&ByteColumn, last_sent_vote, "last_sent_vote_set")?;
        Ok(())
    }

    fn last_voted_set(&mut self, last_voted: &LastVoted) -> Result<(), StorageError> {
        self.db()
            .cf(LastVotedCf)?
            .put(&ByteColumn, last_voted, "last_voted_set")?;
        Ok(())
    }

    fn last_executed_set(&mut self, last_exec: &LastExecuted) -> Result<(), StorageError> {
        self.db()
            .cf(LastExecutedCf)?
            .put(&ByteColumn, last_exec, "last_executed_set")?;

        Ok(())
    }

    fn last_proposed_set(&mut self, last_proposed: &LastProposed) -> Result<(), StorageError> {
        self.db()
            .cf(LastProposedCf)?
            .put(&ByteColumn, last_proposed, "last_proposed_set")?;

        Ok(())
    }

    fn leaf_block_set(&mut self, leaf_node: &LeafBlock) -> Result<(), StorageError> {
        self.db()
            .cf(LeafBlockCf)?
            .put(&ByteColumn, leaf_node, "leaf_block_set")?;

        Ok(())
    }

    fn highest_seen_block_set(&mut self, last_seen_block: &HighestSeenBlock) -> Result<(), StorageError> {
        self.db()
            .cf(HighestSeenBlockCf)?
            .put(&ByteColumn, last_seen_block, "highest_seen_block_set")?;
        Ok(())
    }

    fn last_sent_new_view_set(&mut self, last_sent_new_view: &LastSentNewView) -> Result<(), StorageError> {
        self.db()
            .cf(LastSentNewViewCf)?
            .put(&ByteColumn, last_sent_new_view, "last_sent_new_view_set")?;
        Ok(())
    }

    fn last_sent_new_view_clear(&mut self) -> Result<(), StorageError> {
        self.db()
            .cf(LastSentNewViewCf)?
            .delete(&ByteColumn, "last_sent_new_view_clear")?;
        Ok(())
    }

    fn locked_block_set(&mut self, locked_block: &LockedBlock) -> Result<(), StorageError> {
        self.db()
            .cf(LockedBlockCf)?
            .put(&ByteColumn, locked_block, "locked_block_set")?;

        Ok(())
    }

    fn high_pc_set(&mut self, high_qc: &HighPc) -> Result<(), StorageError> {
        self.db().cf(HighPcCf)?.put(&ByteColumn, high_qc, "high_qc_set")?;
        Ok(())
    }

    fn high_tc_set(&mut self, high_tc: &HighTc) -> Result<(), StorageError> {
        self.db().cf(HighTcCf)?.put(&ByteColumn, high_tc, "high_tc_set")?;
        Ok(())
    }

    fn foreign_proposals_save(&mut self, foreign_proposal: &ForeignProposalRecord) -> Result<(), StorageError> {
        const OPERATION: &str = "foreign_proposals_save";
        let db = self.db();
        let cf = db.cf(ForeignProposalCf)?;

        if cf.exists(foreign_proposal.block_id(), OPERATION)? {
            self.foreign_proposals_set_status(
                foreign_proposal.block_id(),
                foreign_proposal.status(),
                foreign_proposal.proposed_in_block(),
            )?;
        } else {
            cf.put(foreign_proposal.block_id(), foreign_proposal, OPERATION)?;

            db.cf(foreign_proposal::EpochIndex)?.put(
                &(foreign_proposal.epoch(), *foreign_proposal.block_id()),
                &ForeignProposalEpochIndexData {
                    block_id: *foreign_proposal.block_id(),
                    proposed_in_block: foreign_proposal.proposed_in_block().copied(),
                },
                OPERATION,
            )?;
            // Update indexes as required - you cannot use foreign_proposals_set_status because it compares the current
            // record (the one we've just set above) to the changes, which will always be equal, therefore,
            // no indexes will be updated.
            if let Some(proposed_block_id) = foreign_proposal.proposed_in_block() {
                db.cf(foreign_proposal::ProposedInBlockIndex)?.put(
                    &(*proposed_block_id, *foreign_proposal.block_id()),
                    &(),
                    OPERATION,
                )?;
            }

            if foreign_proposal.status().is_unconfirmed() {
                db.cf(foreign_proposal::UnconfirmedIndex)?.put(
                    &(foreign_proposal.epoch(), *foreign_proposal.block_id()),
                    &(),
                    OPERATION,
                )?;
            } else {
                db.cf(foreign_proposal::UnconfirmedIndex)?
                    .delete(&(foreign_proposal.epoch(), *foreign_proposal.block_id()), OPERATION)?;
            }
        }

        Ok(())
    }

    fn foreign_proposals_delete(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        const OPERATION: &str = "foreign_proposals_delete";
        let db = self.db();
        // TODO: to avoid loading the decoded proposal, an block_id -> epoch index could be made
        // We should also consider keeping foreign proposals out of persistence and in memory
        let fp = db.cf(ForeignProposalCf)?.get(block_id, OPERATION)?;
        db.cf(ForeignProposalCf)?.delete(block_id, OPERATION)?;
        db.cf(foreign_proposal::EpochIndex)?
            .delete_or_not_found(&(fp.epoch(), *block_id), OPERATION)?;
        db.cf(foreign_proposal::UnconfirmedIndex)?
            .delete(&(fp.epoch(), *block_id), OPERATION)?;
        if let Some(proposed_block_id) = fp.proposed_in_block() {
            db.cf(foreign_proposal::ProposedInBlockIndex)?
                .delete(&(*proposed_block_id, *fp.block_id()), OPERATION)?;
        }
        Ok(())
    }

    fn foreign_proposals_set_status(
        &mut self,
        block_id: &BlockId,
        status: ForeignProposalStatus,
        set_proposed_in_block: Option<&BlockId>,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "foreign_proposals_set_status";
        let mut fp = self.db().cf(ForeignProposalCf)?.get(block_id, OPERATION)?;
        let db = self.db();

        if fp.status().is_unconfirmed() && !status.is_unconfirmed() {
            db.cf(foreign_proposal::UnconfirmedIndex)?
                .delete(&(fp.epoch(), *block_id), OPERATION)?;
        } else if !fp.status().is_unconfirmed() && status.is_unconfirmed() {
            db.cf(foreign_proposal::UnconfirmedIndex)?
                .put(&(fp.epoch(), *block_id), &(), OPERATION)?;
        } else {
            // no change in unconfirmed status
        }

        fp.set_proposal_status(status);

        if let Some(proposed_in_block) = set_proposed_in_block {
            let index_cf = db.cf(foreign_proposal::ProposedInBlockIndex)?;
            if let Some(prev_id) = fp.proposed_in_block() &&
                prev_id != proposed_in_block
            {
                index_cf.delete(&(*prev_id, *fp.block_id()), OPERATION)?;
            }
            index_cf.put(&(*proposed_in_block, *fp.block_id()), &(), OPERATION)?;

            // Update the epoch index
            let epoch_index_cf = db.cf(foreign_proposal::EpochIndex)?;
            let key = (fp.epoch(), *block_id);
            let mut index = epoch_index_cf.get(&key, OPERATION)?;
            index.proposed_in_block = Some(*proposed_in_block);
            epoch_index_cf.put(&key, &index, OPERATION)?;

            fp.set_proposed_in_block(*proposed_in_block);
        }

        // Update the record
        self.db().cf(ForeignProposalCf)?.put(block_id, &fp, OPERATION)?;

        Ok(())
    }

    fn foreign_proposals_clear_proposed_in(&mut self, proposed_in_block: &BlockId) -> Result<(), StorageError> {
        const OPERATION: &str = "foreign_proposals_clear_proposed_in";
        let db = self.db();

        let cf = db.cf(foreign_proposal::ByProposedInBlockIndexQuery)?;
        let proposed_keys = cf.query_prefix_range_keys(Ordering::default(), proposed_in_block)?;

        for (proposed_in_block, fp_id) in proposed_keys {
            let mut fp = db.cf(ForeignProposalCf)?.get(&fp_id, OPERATION)?;
            if fp.proposed_in_block() == Some(&proposed_in_block) {
                // Setting the status to New in this case
                if !fp.status().is_unconfirmed() {
                    db.cf(foreign_proposal::UnconfirmedIndex)?
                        .put(&(fp.epoch(), *fp.block_id()), &(), OPERATION)?;
                }

                fp.reset_proposed();
                db.cf(ForeignProposalCf)?.put(&fp_id, &fp, OPERATION)?;
            }

            db.cf(foreign_proposal::ProposedInBlockIndex)?
                .delete(&(proposed_in_block, fp_id), OPERATION)?;
        }

        Ok(())
    }

    fn transactions_insert(&mut self, tx_rec: &TransactionRecord) -> Result<(), StorageError> {
        self.db()
            .cf(TransactionCf)?
            .put(tx_rec.id(), tx_rec, "transactions_insert")?;
        Ok(())
    }

    fn transactions_finalize_all<'a, I: IntoIterator<Item = &'a TransactionPoolRecord>>(
        &mut self,
        epoch: Epoch,
        transactions: I,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "transactions_finalize_all";

        let finalized_cf = self.db().cf(FinalizedTransactionLinkCf)?;
        let epoch_index_cf = self.db().cf(finalized_transaction::EpochIndex)?;

        let iter = transactions.into_iter();
        // Add transactions to finalized CF
        let data = FinalizedTransactionLinkData {
            finalized_at: now(),
            epoch,
        };
        for transaction in iter {
            // The epoch index holds exactly one entry per id: a re-finalized id (a previously
            // aborted transaction sequenced again) moves to the epoch it last finalized in, so the
            // GC horizon applies to the latest attempt.
            if let Some(prev) = finalized_cf.get(transaction.id(), OPERATION).optional()? &&
                prev.epoch != epoch
            {
                epoch_index_cf.delete(&(prev.epoch, *transaction.id()), OPERATION)?;
            }
            finalized_cf.put(transaction.id(), &data, OPERATION)?;
            epoch_index_cf.put(&(epoch, *transaction.id()), &(), OPERATION)?;
        }

        Ok(())
    }

    fn transactions_finalized_remove(&mut self, tx_id: &TransactionId) -> Result<(), StorageError> {
        const OPERATION: &str = "transactions_finalized_remove";

        let finalized_cf = self.db().cf(FinalizedTransactionLinkCf)?;
        // The epoch index entry must go with the link, otherwise epoch GC would later delete the
        // payload of an id that has been re-sequenced and is live again.
        if let Some(link) = finalized_cf.get(tx_id, OPERATION).optional()? {
            self.db()
                .cf(finalized_transaction::EpochIndex)?
                .delete(&(link.epoch, *tx_id), OPERATION)?;
        }
        finalized_cf.delete(tx_id, OPERATION)?;

        let exec_cf = self.db().cf(BlockTransactionExecutionCf)?;
        let exec_query = self.db().cf(block_transaction_execution::ByTransactionIdQuery)?;
        let exec_index_cf = self.db().cf(block_transaction_execution::BlockIndex)?;

        for (tx_id, block_id, epoch, height) in exec_query.query_prefix_range_keys(Ordering::default(), tx_id)? {
            exec_cf.delete(&(tx_id, block_id, epoch, height), OPERATION)?;
            exec_index_cf.delete(&(block_id, tx_id, epoch, height), OPERATION)?;
        }

        Ok(())
    }

    fn block_transaction_executions_insert_or_ignore(
        &mut self,
        transaction_execution: &BlockTransactionExecution,
    ) -> Result<bool, StorageError> {
        const OPERATION: &str = "transaction_executions_insert_or_ignore";

        let cf = self.db().cf(BlockTransactionExecutionCf)?;
        if cf.exists(
            &(
                *transaction_execution.transaction_id(),
                *transaction_execution.block_id(),
                transaction_execution.block_epoch(),
                transaction_execution.block_height(),
            ),
            OPERATION,
        )? {
            debug!(
                target: LOG_TARGET,
                "Transaction execution for transaction {} in block {} {} already exists",
                transaction_execution.transaction_id(),
                transaction_execution.block_id(),
                transaction_execution.block_height()
            );
            return Ok(false);
        }

        debug!(
            target: LOG_TARGET,
            "🔧 Inserting transaction execution for transaction {} in block {} {}",
            transaction_execution.transaction_id(),
            transaction_execution.block_id(),
            transaction_execution.block_height()
        );
        cf.put(
            &(
                *transaction_execution.transaction_id(),
                *transaction_execution.block_id(),
                transaction_execution.block_epoch(),
                transaction_execution.block_height(),
            ),
            transaction_execution,
            OPERATION,
        )?;

        self.db().cf(block_transaction_execution::BlockIndex)?.put(
            &(
                *transaction_execution.block_id(),
                *transaction_execution.transaction_id(),
                transaction_execution.block_epoch(),
                transaction_execution.block_height(),
            ),
            &(),
            OPERATION,
        )?;

        Ok(true)
    }

    fn block_transaction_executions_remove_any_by_block_id(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        const OPERATION: &str = "block_transaction_executions_remove_any_by_block_id";

        let query = self.db().cf(block_transaction_execution::ByBlockQuery)?;
        let cf = self.db().cf(BlockTransactionExecutionCf)?;
        let index_cf = self.db().cf(block_transaction_execution::BlockIndex)?;

        for key in query.query_prefix_range_keys(Ordering::default(), block_id)? {
            index_cf.delete(&key, OPERATION)?;
            let (block_id, tx_id, epoch, height) = key;
            cf.delete(&(tx_id, block_id, epoch, height), OPERATION)?;
        }

        Ok(())
    }

    fn block_transaction_executions_lock_any_for_block(&mut self, lock_block: &LeafBlock) -> Result<(), StorageError> {
        const OPERATION: &str = "block_transaction_executions_lock_any_for_block";

        let block_query = self.db().cf(block_transaction_execution::ByBlockQuery)?;
        let tx_query = self.db().cf(block_transaction_execution::ByTransactionIdQuery)?;
        let cf = self.db().cf(BlockTransactionExecutionCf)?;
        let index_cf = self.db().cf(block_transaction_execution::BlockIndex)?;

        // Remove any executions prior to this block - we do this only if this block has an execution (if not, iter will
        // be empty). By the time the block that finalizes a transaction is committed - there will only be one
        // execution.
        for (_, tx_id, locked_epoch, locked_height) in
            block_query.query_prefix_range_keys(Ordering::default(), lock_block.block_id())?
        {
            for (tx_id, block_id, epoch, height) in tx_query.query_prefix_range_keys(Ordering::default(), &tx_id)? {
                // Don't remove for this block or any later blocks
                if (epoch, height) > (locked_epoch, locked_height) {
                    trace!(
                        target: LOG_TARGET,
                        "Skip deleting transaction execution for transaction {} in block {} ({}/{} > {}/{})",
                        tx_id,
                        block_id,
                        epoch,
                        height,
                        locked_epoch,
                        locked_height
                    );
                    continue;
                }
                if block_id == *lock_block.block_id() {
                    continue;
                }
                debug!(
                    target: LOG_TARGET,
                    "Deleting transaction execution for transaction {} in block {} ({}/{} <= {}/{})",
                    tx_id,
                    block_id,
                    epoch,
                    height,
                    locked_epoch,
                    locked_height
                );
                cf.delete(&(tx_id, block_id, epoch, height), OPERATION)?;
                index_cf.delete(&(block_id, tx_id, epoch, height), OPERATION)?;
            }
        }

        Ok(())
    }

    fn transaction_pool_insert_new(
        &mut self,
        tx_id: TransactionId,
        decision: Decision,
        initial_evidence: &Evidence,
        is_ready: bool,
        is_global: bool,
        max_epoch: Epoch,
        transaction_weight: u64,
    ) -> Result<(), StorageError> {
        let value = TransactionPoolRecord::load(
            tx_id,
            initial_evidence.clone(),
            is_global,
            0,
            None,
            TransactionPoolStage::New,
            None,
            decision,
            None,
            None,
            is_ready,
            max_epoch,
            None,
            time::OffsetDateTime::now_utc(),
            None,
            transaction_weight,
            0,
        );

        self.db()
            .cf(TransactionPoolCf)?
            .insert(&tx_id, &value, "transaction_pool_insert_new")?;

        Ok(())
    }

    fn transaction_pool_add_pending_update(
        &mut self,
        block: &LeafBlock,
        update: &TransactionPoolStatusUpdate,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "transaction_pool_add_pending_update";
        let cf = self.db().cf(TransactionPoolStateUpdateCf)?;
        // insert the update
        let value = TransactionPoolStateUpdateData {
            block_id: *block.block_id(),
            block_height: block.height(),
            transaction_id: *update.transaction_id(),
            evidence: update.evidence().clone(),
            transaction_fee: update.transaction_fee(),
            exhaust_burn: update.exhaust_burn(),
            leader_fee: update.leader_fee().cloned(),
            stage: update.stage(),
            local_decision: update.decision(),
            remote_decision: update.remote_decision(),
            locked_epoch: update.locked_epoch().cloned(),
            is_ready: update.is_ready(),
        };

        cf.put(&(*block.block_id(), *update.transaction_id()), &value, OPERATION)?;

        if self.options.debugging_data {
            let cf = self
                .db()
                .cf(transaction_pool_state_update::TransactionPoolStateUpdateDebugHistoryCf)?;
            cf.put(
                &(block.epoch(), block.height(), *update.transaction_id()),
                &value,
                OPERATION,
            )?;
        }

        // Update the last_updated timestamp on the base record for informational purposes.
        // NOTE: We intentionally do NOT eagerly write pending_stage or is_ready to the base record here.
        // The pending state is resolved from the pending chain when needed (via get_for_blocks / get_many_ready).
        // Eagerly writing these fields caused stale state when blocks ended up on dead branches (leader failures),
        // leading to permanent "Stage disagreement" no-votes.
        let cf = self.db().cf(TransactionPoolCf)?;
        let mut tx_pool_value = cf.get(update.transaction_id(), OPERATION)?;
        tx_pool_value.set_last_updated(*block.block_id(), time::OffsetDateTime::now_utc());
        cf.put(update.transaction_id(), &tx_pool_value, OPERATION)?;

        Ok(())
    }

    fn transaction_pool_remove_all<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<Vec<TransactionPoolRecord>, StorageError> {
        const OPERATION: &str = "transaction_pool_remove_all";

        let cf = self.db().cf(TransactionPoolCf)?;
        let pool_recs = cf.multi_get(transaction_ids, OPERATION)?;
        for tx in &pool_recs {
            cf.delete(tx.id(), OPERATION)?;
        }

        Ok(pool_recs)
    }

    fn transaction_pool_confirm_all_transitions(&mut self, block: &LeafBlock) -> Result<(), StorageError> {
        const OPERATION: &str = "transaction_pool_confirm_all_transitions";

        let by_block_query = self.db().cf(transaction_pool_state_update::ByBlockIdQuery)?;

        let entries = by_block_query.query_prefix_range_entries(block.block_id(), Ordering::Ascending)?;

        let updates_cf = self.db().cf(TransactionPoolStateUpdateCf)?;
        let pool_cf = self.db().cf(TransactionPoolCf)?;
        for (key, update) in entries {
            updates_cf.delete(&key, OPERATION)?;

            // Update the transaction pool record accordingly
            let (_, transaction_id) = &key;
            let mut pool = pool_cf.get(transaction_id, OPERATION)?;
            pool.set_stage(update.stage);
            pool.set_pending_stage(None);
            pool.set_local_decision(update.local_decision);
            pool.set_transaction_fee(update.transaction_fee);
            pool.set_exhaust_burn(update.exhaust_burn);
            if let Some(leader_fee) = update.leader_fee {
                pool.set_leader_fee(leader_fee);
            }
            pool.set_evidence(update.evidence.clone());
            pool.set_is_ready(update.is_ready);
            pool.set_locked_epoch(update.locked_epoch);
            if let Some(remote_decision) = update.remote_decision {
                pool.set_remote_decision(remote_decision);
            }

            pool_cf.put(transaction_id, &pool, OPERATION)?;
        }

        Ok(())
    }

    fn transaction_pool_state_updates_remove_any_by_block_id(
        &mut self,
        block_id: &BlockId,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "transaction_pool_state_updates_remove_any_by_block_id";
        let by_block_query = self.db().cf(transaction_pool_state_update::ByBlockIdQuery)?;

        let keys = by_block_query.query_prefix_range_keys(Ordering::Ascending, block_id)?;

        let updates_cf = self.db().cf(TransactionPoolStateUpdateCf)?;
        for key in keys {
            updates_cf.delete(&key, OPERATION)?;
        }

        Ok(())
    }

    fn parked_block_insert<'a, IMissing: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        block: &Block,
        foreign_proposals: &[ForeignProposal],
        missing_transaction_ids: IMissing,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "missing_transactions_insert";
        let mut missing_transaction_ids = missing_transaction_ids.into_iter().peekable();
        // If there are no missing transactions, then the block should not be parked/will never be unparked
        if missing_transaction_ids.peek().is_none() {
            return Err(StorageError::QueryError {
                reason: "missing_transactions_insert: No missing transactions to insert".to_string(),
            });
        }

        self.parked_blocks_insert(block, foreign_proposals)?;

        let cf = self.db().cf(MissingTransactionCf)?;
        let index_cf = self.db().cf(missing_transactions::MissingTransactionBlockIdIndex)?;
        let values = missing_transaction_ids.map(|tx_id| ((*tx_id, *block.id()), ()));
        for (k, v) in values {
            cf.put(&k, &v, OPERATION)?;
            let (tx_id, block_id) = k;
            index_cf.put(&(block_id, tx_id), &(), OPERATION)?;
        }

        Ok(())
    }

    fn parked_block_remove_missing_transaction(
        &mut self,
        _current_height: NodeHeight,
        transaction_id: &TransactionId,
    ) -> Result<Option<(Block, Vec<ForeignProposal>)>, StorageError> {
        const OPERATION: &str = "missing_transactions_insert";

        let query_cf = self.db().cf(missing_transactions::ByTransactionIdQuery)?;

        // Safe to iterate lazily: the iterator is dropped before anything is written
        #[allow(clippy::disallowed_methods)]
        let mut iter = query_cf.query_prefix_range_key_iterator(Ordering::Ascending, transaction_id);

        let Some(key) = iter.next().transpose()? else {
            return Ok(None);
        };
        drop(iter);

        let cf = self.db().cf(MissingTransactionCf)?;
        cf.delete(&key, OPERATION)?;

        let (_, block_id) = key;

        self.db()
            .cf(missing_transactions::MissingTransactionBlockIdIndex)?
            .delete_or_not_found(&(block_id, *transaction_id), OPERATION)?;

        {
            let query = self.db().cf(missing_transactions::ByBlockIdQuery)?;
            // Safe to iterate lazily: this scope only reads
            #[allow(clippy::disallowed_methods)]
            let mut iter = query.query_prefix_range_key_iterator(Ordering::default(), &block_id);

            // Are there more missing transactions for this block?
            if iter.next().transpose()?.is_some() {
                return Ok(None);
            }
        }

        // TODO: we do not clear older blocks (height < current block height). This could potentially leave stale
        // entries.

        // None left, remove and return the block
        let block_and_fp = self.parked_blocks_remove(&block_id)?;
        Ok(Some(block_and_fp))
    }

    fn foreign_parked_blocks_insert(&mut self, park_block: &ForeignParkedProposal) -> Result<(), StorageError> {
        const OPERATION: &str = "foreign_parked_blocks_insert";
        self.db()
            .cf(ForeignParkedBlockCf)?
            .put(park_block.block_id(), park_block, OPERATION)?;
        Ok(())
    }

    fn foreign_parked_blocks_insert_missing_transactions<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        park_block_id: &BlockId,
        missing_transaction_ids: I,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "foreign_parked_blocks_insert_missing_transactions";
        let parked_cf = self.db().cf(ForeignParkedBlockCf)?;
        if !parked_cf.exists(park_block_id, OPERATION)? {
            return Err(StorageError::QueryError {
                reason: format!(
                    "{}: Cannot insert missing transactions for non-existent parked block {}",
                    OPERATION, park_block_id
                ),
            });
        }

        let cf = self.db().cf(foreign_parked_blocks::MissingTransactionsModel)?;
        let index_cf = self.db().cf(foreign_parked_blocks::MissingTransactionsBlockIdIndex)?;
        for tx_id in missing_transaction_ids {
            cf.put(&(*tx_id, *park_block_id), &(), OPERATION)?;
            index_cf.put(&(*park_block_id, *tx_id), &(), OPERATION)?;
        }
        Ok(())
    }

    fn foreign_parked_blocks_remove_all_by_transaction(
        &mut self,
        transaction_id: &TransactionId,
    ) -> Result<Vec<ForeignParkedProposal>, StorageError> {
        const OPERATION: &str = "foreign_parked_blocks_remove_all_by_transaction";
        let cf = self.db().cf(ForeignParkedBlockCf)?;
        let query = self.db().cf(foreign_parked_blocks::ByTransactionIdQuery)?;
        let missing_cf = self.db().cf(foreign_parked_blocks::MissingTransactionsModel)?;
        let missing_index_cf = self.db().cf(foreign_parked_blocks::MissingTransactionsBlockIdIndex)?;
        let by_block_query = self.db().cf(foreign_parked_blocks::ByBlockIdQuery)?;
        let keys = query.query_prefix_range_keys(Ordering::default(), transaction_id)?;

        // Remove the transaction ids from the missing list
        let mut block_ids = HashSet::new();
        for (transaction_id, block_id) in keys {
            block_ids.insert(block_id);
            missing_cf.delete(&(transaction_id, block_id), OPERATION)?;
            missing_index_cf.delete(&(block_id, transaction_id), OPERATION)?;
        }

        // Only blocks with no missing transactions left are unparked
        let mut unparked = Vec::with_capacity(block_ids.len());
        for block_id in block_ids {
            if !by_block_query.exists_prefix(&block_id)? {
                unparked.push(block_id);
            }
        }

        if unparked.is_empty() {
            return Ok(vec![]);
        }

        // Unpark (fetch and delete) the blocks
        let blocks = cf.multi_get(&unparked, OPERATION)?;
        for id in &unparked {
            cf.delete(id, OPERATION)?;
        }

        Ok(blocks)
    }

    fn substate_locks_insert_all<'a, I: IntoIterator<Item = (&'a SubstateId, &'a Vec<SubstateLock>)>>(
        &mut self,
        block: &LeafBlock,
        locks: I,
    ) -> Result<(), StorageError> {
        self.substate_locks_insert_for_block(block.block_id(), block.epoch(), block.height(), locks)
    }

    fn substate_locks_remove_many_for_transactions<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "substate_locks_remove_many_for_transactions";
        let transaction_ids = {
            let pending = self.pending_state();
            transaction_ids
                .into_iter()
                .filter(|id| pending.locks.holds_locks(id))
                .copied()
                .collect::<Vec<_>>()
        };
        if transaction_ids.is_empty() {
            return Ok(());
        }

        for block_id in self.staged().locks_mut().release(&transaction_ids) {
            self.write_block_lock_set(&block_id, OPERATION)?;
        }

        Ok(())
    }

    fn substate_locks_remove_any_by_block_id(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        const OPERATION: &str = "substate_locks_remove_any_by_block_id";
        self.remove_block_lock_set(block_id, OPERATION)
    }

    fn substates_commit_batch(&mut self, update_batch: SubstateUpdateBatch) -> Result<(), StorageError> {
        const OPERATION: &str = "substates_commit_batch";

        let db = self.db();

        let cf = db.cf(SubstateCf)?;
        let head_cf = db.cf(substate::HeadIndex)?;
        let unpruned_cf = db.cf(substate::UnprunedDownedValuesIndex)?;

        for (shard, updates) in update_batch.updates {
            for (state_version, updates) in updates {
                let mut transitions = Vec::with_capacity(updates.len());
                let mut downed_substate_addresses = vec![];

                for transition in updates {
                    match transition {
                        SubstateTransition::Up {
                            id,
                            version,
                            substate_or_hash,
                        } => {
                            let rec = SubstateRecord::new(
                                update_batch.network,
                                id,
                                version,
                                substate_or_hash,
                                SubstateCreated {
                                    at_epoch: update_batch.epoch,
                                    in_shard: shard,
                                    at_state_version: state_version,
                                },
                            );

                            let address = rec.to_substate_address();
                            cf.put(&address, &rec, OPERATION)?;
                            head_cf.put(rec.substate_id(), &SubstateHeadData { version, is_up: true }, OPERATION)?;

                            transitions.push(StateTransitionRecordData {
                                substate_address: address,
                                transition: StateTransitionType::Up,
                            });
                        },
                        SubstateTransition::Down { id } => {
                            let address = id.to_substate_address();

                            let mut substate = cf.get_for_update(&address, OPERATION)?;
                            substate.set_destroyed(SubstateDestroyed {
                                at_epoch: update_batch.epoch,
                                at_state_version: state_version,
                            });
                            cf.put(&address, &substate, OPERATION)?;
                            head_cf.put(
                                &substate.substate_id,
                                &SubstateHeadData {
                                    version: substate.version(),
                                    is_up: false,
                                },
                                OPERATION,
                            )?;
                            downed_substate_addresses.push(address);

                            transitions.push(StateTransitionRecordData {
                                substate_address: address,
                                transition: StateTransitionType::Down,
                            });
                        },
                    }
                }

                // Take note of downed substates for pruning in a separate task
                if !downed_substate_addresses.is_empty() {
                    unpruned_cf.put(
                        &(update_batch.epoch, shard, state_version),
                        &downed_substate_addresses,
                        OPERATION,
                    )?;
                }

                let transition = StateTransitionModelDataV1 {
                    epoch: update_batch.epoch,
                    state_version,
                    transitions,
                };

                db.cf(StateTransitionCf)?
                    .put(&(shard, state_version), &transition, OPERATION)?;
            }
        }

        Ok(())
    }

    fn substates_prune_downed_values(&mut self, epoch: Epoch, limit: usize) -> Result<usize, StorageError> {
        const OPERATION: &str = "substates_prune_downed_values";
        let db = self.db();
        let unpruned_query = db.cf(substate::UnprunedDownedValuesEpochQuery)?;
        let unpruned_index = db.cf(substate::UnprunedDownedValuesIndex)?;
        let substates_cf = db.cf(SubstateCf)?;

        // Every entry holds at least one address, so no more than `limit` entries are needed to reach the limit.
        let mut entries =
            unpruned_query.query_end_range_entries_limited(Ordering::Ascending, &(epoch + Epoch(1)), limit)?;
        let mut count = 0usize;
        let mut num_entries = 0usize;
        for (_, addresses) in &entries {
            if count >= limit {
                break;
            }
            count += addresses.len();
            num_entries += 1;
        }
        entries.truncate(num_entries);

        for (key, addresses) in entries {
            // TODO(perf): consider storing the actual values in a separate column family to avoid get/set
            for substate_addr in addresses {
                let mut substate = substates_cf.get_for_update(&substate_addr, OPERATION)?;
                substate.clear_substate_value();
                substates_cf.put(&substate_addr, &substate, OPERATION)?;
            }
            unpruned_index.delete(&key, OPERATION)?;
        }

        Ok(count)
    }

    fn substates_rewind_to_state_version(
        &mut self,
        shard: Shard,
        target_state_version: Version,
    ) -> Result<SubstateRewindStats, StorageError> {
        const OPERATION: &str = "substates_rewind_to_state_version";

        let db = self.db();
        let transitions_cf = db.cf(StateTransitionCf)?;
        let transitions_query = db.cf(state_transition::ByShardAndStateVersionQuery)?;
        let substates_cf = db.cf(SubstateCf)?;
        let head_cf = db.cf(substate::HeadIndex)?;
        let unpruned_cf = db.cf(substate::UnprunedDownedValuesIndex)?;
        let down_proofs_cf = db.cf(SubstateDownProofCf)?;

        let start_version = target_state_version.saturating_add(1);
        let mut touched: HashSet<SubstateId> = HashSet::new();
        let mut stats = SubstateRewindStats::default();

        // Collect only the versions to process, then load each record one at a time inside the loop. This caps memory
        // at ~O(n_versions) rather than O(n_versions * avg_record_size), which matters for rewinds spanning many
        // epochs.
        let versions =
            transitions_query.query_range_keys(Ordering::Descending, (shard, start_version)..(shard, Version::MAX))?;

        // Descending, so `versions` is already in reverse state_version order.
        for key in versions {
            debug_assert_eq!(key.0, shard, "range iterator leaked across shard boundary");
            let record = transitions_cf.get(&key, OPERATION)?;
            // Within a record, invert transitions in reverse index order so that the inverse sequence is the exact
            // time-reversed mirror of forward application.
            for transition in record.transitions.iter().rev() {
                let address = &transition.substate_address;
                match transition.transition {
                    StateTransitionType::Up => {
                        let substate = substates_cf.get(address, OPERATION)?;
                        touched.insert(substate.substate_id.clone());
                        substates_cf.delete(address, OPERATION)?;
                        stats.substates_created_deleted += 1;
                    },
                    StateTransitionType::Down => {
                        let mut substate = substates_cf.get(address, OPERATION)?;
                        touched.insert(substate.substate_id.clone());
                        substate.destroyed = None;
                        substates_cf.put(address, &substate, OPERATION)?;
                        down_proofs_cf.delete(&(shard, *address), OPERATION).optional()?;
                        stats.substates_destroyed_restored += 1;
                    },
                }
            }

            // Remove the transition record and any unpruned-down index entry for this (epoch, shard, version).
            transitions_cf.delete(&key, OPERATION)?;
            unpruned_cf
                .delete(&(record.epoch, key.0, key.1), OPERATION)
                .optional()?;
            stats.transitions_processed += 1;
        }

        // Rebuild the head index for every touched SubstateId by finding the highest surviving version via a reverse
        // prefix scan on SubstateCf. SubstateAddress = object_key || version_be, so all versions for a given
        // substate_id are lexically adjacent.
        for substate_id in touched {
            let object_key = substate_id.to_object_key();
            let object_key_bytes: &[u8] = object_key.as_ref();
            let mut raw_prefix = Vec::with_capacity(1 + object_key_bytes.len());
            raw_prefix.push(KeyPrefix::Substates.as_u8());
            raw_prefix.extend_from_slice(object_key_bytes);

            let highest = substates_cf
                .prefix_range_iterator_raw_key(Ordering::Descending, raw_prefix)
                .next()
                .transpose()?;
            match highest {
                Some((_address, record)) => {
                    head_cf.put(
                        &substate_id,
                        &SubstateHeadData {
                            version: record.version,
                            is_up: record.is_up(),
                        },
                        OPERATION,
                    )?;
                },
                None => {
                    head_cf.delete(&substate_id, OPERATION).optional()?;
                },
            }
            stats.heads_updated += 1;
        }

        debug!(
            target: LOG_TARGET,
            "🔄 Rewound substates for shard {shard} to state_version {target_state_version}: {} transitions, {} \
             created-deleted, {} destroyed-restored, {} heads updated",
            stats.transitions_processed,
            stats.substates_created_deleted,
            stats.substates_destroyed_restored,
            stats.heads_updated,
        );

        Ok(stats)
    }

    fn foreign_substate_pledges_save(
        &mut self,
        transaction_id: &TransactionId,
        // This is a field used in the SQL implementation for debugging
        _shard_group: ShardGroup,
        pledges: &SubstatePledges,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "foreign_substate_pledges_save";

        let cf = self.db().cf(ForeignSubstatePledgeCf)?;
        for pledge in pledges {
            let key = (*transaction_id, pledge.to_substate_address());
            cf.put(&key, pledge, OPERATION)?;
        }

        Ok(())
    }

    fn foreign_substate_pledges_remove_many<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "foreign_substate_pledges_remove_many";

        let cf = self.db().cf(ForeignSubstatePledgeCf)?;
        let query = self.db().cf(foreign_substate_pledge::ByTransactionIdQuery)?;

        for transaction_id in transaction_ids {
            for key in query.query_prefix_range_keys(Ordering::default(), transaction_id)? {
                cf.delete(&key, OPERATION)?;
            }
        }

        Ok(())
    }

    fn pending_state_tree_diffs_insert_all<'a, I: IntoIterator<Item = (&'a Shard, &'a PendingShardStateTreeDiff)>>(
        &mut self,
        block_id: &BlockId,
        diffs: I,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "pending_state_tree_diffs_insert_all";
        let diffs = diffs
            .into_iter()
            .map(|(shard, diff)| ShardStateTreeDiffRef { shard: *shard, diff })
            .collect::<Vec<_>>();
        if diffs.is_empty() {
            return self.pending_state_tree_diffs_remove_by_block(block_id);
        }
        trace!(
            target: LOG_TARGET,
            "{OPERATION}: {} shard(s) in block {}", diffs.len(), block_id
        );

        let encoded = tari_bor::encode(&diffs).map_err(|e| RocksDbStorageError::EncodeError { source: e.into() })?;
        self.db()
            .cf(PendingStateTreeDiffRecordCf)?
            .put_raw_value(block_id, &encoded, OPERATION)?;
        let entry = diffs
            .into_iter()
            .map(|ShardStateTreeDiffRef { shard, diff }| ShardStateTreeDiff {
                shard,
                diff: diff.clone(),
            })
            .collect();
        self.staged().tree_diffs_mut().insert(*block_id, entry);
        Ok(())
    }

    fn pending_state_tree_diffs_remove_by_block(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        const OPERATION: &str = "pending_state_tree_diffs_remove_by_block";
        if self.pending_state().tree_diffs.get(block_id).is_none() {
            return Ok(());
        }
        self.staged().tree_diffs_mut().remove(block_id);
        self.db()
            .cf(PendingStateTreeDiffRecordCf)?
            .delete(block_id, OPERATION)?;
        Ok(())
    }

    fn pending_state_tree_diffs_remove_and_return_by_block(
        &mut self,
        block_id: &BlockId,
    ) -> Result<IndexMap<Shard, Vec<PendingShardStateTreeDiff>>, StorageError> {
        const OPERATION: &str = "pending_state_tree_diffs_remove_and_return_by_block";
        if self.pending_state().tree_diffs.get(block_id).is_none() {
            return Ok(IndexMap::new());
        }
        let Some(entry) = self.staged().tree_diffs_mut().remove(block_id) else {
            return Ok(IndexMap::new());
        };
        self.db()
            .cf(PendingStateTreeDiffRecordCf)?
            .delete(block_id, OPERATION)?;

        let mut diffs = IndexMap::<Shard, Vec<PendingShardStateTreeDiff>>::new();
        for ShardStateTreeDiff { shard, diff } in entry.iter() {
            diffs.entry(*shard).or_default().push(diff.clone());
        }
        Ok(diffs)
    }

    fn state_tree_nodes_batch_insert(
        &mut self,
        shard: Shard,
        nodes: &[(NodeKey, Node<StateTreePayload>)],
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "state_tree_nodes_insert";
        let cf = self.db().cf(StateTreeCf)?;
        for (key, node) in nodes {
            cf.put(&(shard, key.clone()), node, OPERATION)?;
        }
        Ok(())
    }

    fn state_tree_nodes_record_stale_tree_nodes(
        &mut self,
        shard: Shard,
        version: Version,
        nodes: &[StaleTreeNode],
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "state_tree_nodes_record_stale_tree_nodes";

        let encoded = StaleTreeNodesCodec.encode_slice(nodes)?;
        self.db()
            .cf(StateTreeStaleNodesCf)?
            .put_raw_value(&(shard, version), &encoded, OPERATION)?;

        Ok(())
    }

    fn state_tree_nodes_clear_stale(
        &mut self,
        num_preshards: NumPreshards,
        max_deletes: usize,
    ) -> Result<usize, StorageError> {
        const OPERATION: &str = "state_tree_nodes_clear_stale";

        let cf = self.db().cf(StateTreeCf)?;
        let versions_cf = self.db().cf(StateTreeShardVersionCf)?;
        let stale_cf = self.db().cf(state_tree::ByStateTreeStaleShardVersionQuery)?;
        let stale_nodes_cf = self.db().cf(StateTreeStaleNodesCf)?;

        let mut num_deleted = 0usize;
        let shards = iter::once(Shard::global()).chain(ShardGroup::all_shards(num_preshards).shard_iter());
        for shard in shards {
            if num_deleted >= max_deletes {
                break;
            }
            let max_version = versions_cf.get(&shard, OPERATION).optional()?.unwrap_or(0);
            let Some(to_version) = max_version.checked_sub(self.options.state_history_length) else {
                trace!(target: LOG_TARGET, "Shard {shard} is at version {max_version}, skipping stale node deletion due to history length {}", self.options.state_history_length);
                continue;
            };
            // Only the keys are taken up front: this loop deletes the stale-node record it is reading, and a write
            // transaction's iterator must not be written through at the key it is standing on. Every version deletes
            // at least its own record, so no more than the remaining budget of versions can be processed.
            let stale_keys = stale_cf.query_range_keys_limited(
                Ordering::Ascending,
                (shard, 0)..(shard, to_version.saturating_add(1)),
                max_deletes - num_deleted,
            )?;
            for (shard, version) in stale_keys {
                if num_deleted >= max_deletes {
                    break;
                }
                let nodes = stale_nodes_cf.get(&(shard, version), OPERATION)?;

                // A version's nodes are gathered before any is deleted, because the subtree walk reads the nodes it
                // descends through.
                let mut delete_buffer = Vec::new();
                for node in nodes {
                    match node {
                        StaleTreeNode::Node(key) => {
                            trace!(target: LOG_TARGET, "Deleting stale node {key} from shard {shard}", );
                            delete_buffer.push((shard, key));
                        },
                        StaleTreeNode::Subtree(parent_key) => {
                            trace!(target: LOG_TARGET, "Deleting stale substree {parent_key} from shard {shard}", );
                            // A subtree already deleted along with an earlier version is skipped.
                            let Some(parent_node) = cf.get(&(shard, parent_key.clone()), OPERATION).optional()? else {
                                continue;
                            };

                            match parent_node {
                                Node::Internal(node) => {
                                    delete_buffer.extend(recurse_subtree_depth_first_post_order(
                                        &cf,
                                        shard,
                                        parent_key,
                                        node.into_children(),
                                    ));
                                },
                                Node::Leaf(_) => {
                                    trace!(target: LOG_TARGET, "Deleting stale leaf node {parent_key} from shard {shard}", );
                                    delete_buffer.push((shard, parent_key));
                                },
                                Node::Null => {},
                            }
                        },
                    }
                }

                for key in &delete_buffer {
                    cf.delete(key, OPERATION)?;
                }
                // The record goes in the same transaction as its nodes, so a version is never left half-cleared.
                stale_nodes_cf.delete(&(shard, version), OPERATION)?;
                num_deleted += delete_buffer.len() + 1;
            }
        }

        Ok(num_deleted)
    }

    fn state_tree_shard_versions_set(&mut self, shard: Shard, version: Version) -> Result<(), StorageError> {
        const OPERATION: &str = "state_tree_shard_versions_set";

        self.db()
            .cf(StateTreeShardVersionCf)?
            .put(&shard, &version, OPERATION)?;

        Ok(())
    }

    fn state_tree_truncate_to_version(
        &mut self,
        shard: Shard,
        target_version: Version,
    ) -> Result<StateTreeTruncateStats, StorageError> {
        const OPERATION: &str = "state_tree_truncate_to_version";

        let db = self.db();
        let tree_cf = db.cf(StateTreeCf)?;
        let version_query = db.cf(state_tree::ByShardStateVersionQuery)?;
        let stale_query = db.cf(state_tree::ByStateTreeStaleShardVersionQuery)?;
        let stale_cf = db.cf(StateTreeStaleNodesCf)?;
        let versions_cf = db.cf(StateTreeShardVersionCf)?;

        // Versions are u64; saturate on the unlikely MAX case so we don't overflow.
        let start_version = target_version.saturating_add(1);

        // 1. Delete tree nodes at versions > target for this shard.
        let node_keys =
            version_query.query_range_keys(Ordering::Ascending, (shard, start_version)..(shard, Version::MAX))?;
        for key in &node_keys {
            debug_assert_eq!(key.0, shard, "range iterator leaked across shard boundary");
            tree_cf.delete(key, OPERATION)?;
        }

        // 2. Delete stale-node records at versions > target for this shard.
        let stale_keys =
            stale_query.query_range_keys(Ordering::Ascending, (shard, start_version)..(shard, Version::MAX))?;
        for key in &stale_keys {
            debug_assert_eq!(key.0, shard, "range iterator leaked across shard boundary");
            stale_cf.delete(key, OPERATION)?;
        }

        // 3. Delete the proofs of versions > target for this shard: they prove state that no longer exists.
        let proofs_cf = db.cf(StateVersionProofCf)?;
        let proof_keys = proofs_cf
            .range_iterator(
                Ordering::Ascending,
                proofs_cf.encode_key(&(shard, start_version))..proofs_cf.encode_key(&(shard, Version::MAX)),
            )
            .map(|res| res.map(|(key, _)| key))
            .collect::<Result<Vec<_>, _>>()?;
        for key in &proof_keys {
            proofs_cf.delete(key, OPERATION)?;
        }

        // 4. Reset the latest-version pointer to the highest version with surviving tree nodes. The pointer must
        //    reference a version at which a JMT root node exists — downstream readers (JMT root lookup, state sync's
        //    root check) load the root at Some(v) and treat a missing entry as the empty tree (placeholder hash).
        //    Version 0 is a valid committed version: genesis substates are bootstrapped into the tree at version 0, so
        //    a shard whose only state is genesis must keep a pointer at 0 or it reads as empty and no longer matches
        //    the checkpoint being rolled back to.
        let latest_surviving_version = version_query
            .query_range_keys_limited(Ordering::Descending, (shard, 0)..(shard, start_version), 1)?
            .first()
            .map(|(_, node_key)| node_key.version());
        match latest_surviving_version {
            Some(version) => versions_cf.put(&shard, &version, OPERATION)?,
            None => versions_cf.delete(&shard, OPERATION)?,
        }

        debug!(
            target: LOG_TARGET,
            "Truncated state tree for shard {shard} to version {target_version}: deleted {} node(s), {} stale \
             record(s)",
            node_keys.len(),
            stale_keys.len(),
        );

        Ok(StateTreeTruncateStats {
            nodes_deleted: node_keys.len(),
            stale_records_deleted: stale_keys.len(),
        })
    }

    fn state_sync_rewind_point_set(&mut self, shard: Shard, version: Version) -> Result<(), StorageError> {
        const OPERATION: &str = "state_sync_rewind_point_set";
        self.db().cf(StateSyncRewindPointCf)?.put(&shard, &version, OPERATION)?;
        Ok(())
    }

    fn state_version_proofs_insert(&mut self, proof: &StateVersionProof) -> Result<(), StorageError> {
        const OPERATION: &str = "state_version_proofs_insert";
        self.db()
            .cf(StateVersionProofCf)?
            .put(&(proof.shard, proof.state_version), proof, OPERATION)?;
        Ok(())
    }

    fn block_commit_proofs_insert(&mut self, block_id: &BlockId, commit_proof: &[u8]) -> Result<(), StorageError> {
        const OPERATION: &str = "block_commit_proofs_insert";
        self.db()
            .cf(BlockCommitProofCf)?
            .put_raw_value(block_id, commit_proof, OPERATION)?;
        Ok(())
    }

    fn substate_down_proofs_insert(
        &mut self,
        shard: Shard,
        id: &VersionedSubstateId,
        record: &SubstateDownProofRecord,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "substate_down_proofs_insert";
        self.db()
            .cf(SubstateDownProofCf)?
            .put(&(shard, id.to_substate_address()), record, OPERATION)?;
        Ok(())
    }

    fn state_sync_rewind_point_remove(&mut self, shard: Shard) -> Result<(), StorageError> {
        const OPERATION: &str = "state_sync_rewind_point_remove";
        self.db().cf(StateSyncRewindPointCf)?.delete(&shard, OPERATION)?;
        Ok(())
    }

    fn epoch_checkpoint_save(&mut self, checkpoint: &EpochCheckpoint) -> Result<(), StorageError> {
        const OPERATION: &str = "epoch_checkpoint_save";
        let shard_group = checkpoint.checked_shard_group().map_err(|e| StorageError::QueryError {
            reason: format!(
                "{OPERATION}: Invalid shard group for epoch {}: {}",
                checkpoint.epoch(),
                e
            ),
        })?;
        self.db()
            .cf(EpochCheckpointCf)?
            .put(&(checkpoint.epoch(), shard_group), checkpoint, OPERATION)?;

        Ok(())
    }

    fn lock_conflicts_insert_all<'a, I: IntoIterator<Item = (&'a TransactionId, &'a Vec<LockConflict>)>>(
        &mut self,
        block_id: &BlockId,
        conflicts: I,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "lock_conflicts_insert_all";

        let cf = self.db().cf(LockConflictCf)?;
        let index_cf = self.db().cf(lock_conflict::LockConflictBlockIdIndex)?;
        for (tx_id, conflicts) in conflicts {
            for conflict in conflicts {
                cf.put(&(*tx_id, *block_id, conflict.transaction_id), conflict, OPERATION)?;
                index_cf.put(&(*block_id, *tx_id, conflict.transaction_id), &(), OPERATION)?;
            }
        }

        Ok(())
    }

    fn lock_conflicts_remove_by_transaction_ids<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "lock_conflicts_remove_by_transaction_ids";
        let mut transaction_ids = transaction_ids.into_iter().peekable();
        if transaction_ids.peek().is_none() {
            return Ok(());
        }

        let db = self.db();
        let cf = db.cf(LockConflictCf)?;
        let index_cf = db.cf(lock_conflict::LockConflictBlockIdIndex)?;
        let query = db.cf(lock_conflict::ByTransactionIdQuery)?;

        for tx_id in transaction_ids {
            for key in query.query_prefix_range_keys(Ordering::Ascending, tx_id)? {
                cf.delete(&key, OPERATION)?;
                // Delete if the dependent transaction and depending transaction are swapped
                let (transaction_id, block_id, depends_on_tx_id) = key;
                cf.delete(&(depends_on_tx_id, block_id, transaction_id), OPERATION)?;
                index_cf.delete(&(block_id, transaction_id, depends_on_tx_id), OPERATION)?;
                index_cf.delete(&(block_id, depends_on_tx_id, transaction_id), OPERATION)?;
            }
        }

        Ok(())
    }

    fn lock_conflicts_remove_by_block_id(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        const OPERATION: &str = "lock_conflicts_remove_by_block_id";

        let cf = self.db().cf(LockConflictCf)?;
        let query_cf = self.db().cf(lock_conflict::ByBlockIdQuery)?;
        let index_cf = self.db().cf(lock_conflict::LockConflictBlockIdIndex)?;

        for key in query_cf.query_prefix_range_keys(Ordering::Ascending, block_id)? {
            index_cf.delete(&key, OPERATION)?;
            let (block_id, transaction_id, depends_on_tx) = key;
            cf.delete(&(transaction_id, block_id, depends_on_tx), OPERATION)?;
            cf.delete(&(depends_on_tx, block_id, transaction_id), OPERATION)?;
        }

        Ok(())
    }

    fn validator_epoch_stats_updates<'a, I: IntoIterator<Item = ValidatorStatsUpdate<'a>>>(
        &mut self,
        epoch: Epoch,
        committed_height: NodeHeight,
        updates: I,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "validator_epoch_stats_updates";

        let cf = self.db().cf(ValidatorNodeEpochStatsCf)?;
        let log_cf = self.db().cf(ValidatorLivenessLogCf)?;
        for update in updates {
            let key = (epoch, *update.public_key());
            let mut stats = cf.get(&key, OPERATION).optional()?.unwrap_or_default();
            let liveness_changed = stats.apply(&update, committed_height);
            cf.put(&key, &stats, OPERATION)?;
            if liveness_changed {
                log_cf.put(
                    &(epoch, *update.public_key(), committed_height),
                    &stats.counters(),
                    OPERATION,
                )?;
            }
        }

        Ok(())
    }

    fn epoch_cleanup_step(
        &mut self,
        epoch: Epoch,
        step: EpochCleanupStep,
        limit: usize,
    ) -> Result<usize, StorageError> {
        let Some(prune_epoch) = epoch.checked_sub(self.options.epoch_history_length) else {
            return Ok(0);
        };

        let db = self.db();
        match step {
            EpochCleanupStep::DownedSubstateValues => self.substates_prune_downed_values(prune_epoch, limit),
            EpochCleanupStep::Blocks => cleanup::blocks_for_epoch(&db, prune_epoch, limit),
            EpochCleanupStep::ProposalCertificates => cleanup::proposal_certificates_for_epoch(&db, prune_epoch, limit),
            EpochCleanupStep::TimeoutCertificates => cleanup::timeout_certificates_for_epoch(&db, prune_epoch, limit),
            EpochCleanupStep::VoteEquivocations => cleanup::vote_equivocations_for_epoch(&db, prune_epoch, limit),
            EpochCleanupStep::ValidatorLivenessLog => {
                cleanup::validator_liveness_log_for_epoch(&db, prune_epoch, limit)
            },
            EpochCleanupStep::ForeignProposals => cleanup::foreign_proposals_for_epoch(&db, prune_epoch, limit),
            EpochCleanupStep::FinalizedTransactions => {
                if self.options.prune_transaction_history {
                    cleanup::finalized_transactions_for_epoch(&db, prune_epoch, limit)
                } else {
                    Ok(0)
                }
            },
        }
    }

    fn vote_equivocation_record(&mut self, evidence: &VoteEquivocation) -> Result<bool, StorageError> {
        const OPERATION: &str = "vote_equivocation_record";
        let key = (evidence.epoch, evidence.height, evidence.public_key);
        let cf = self.db().cf(vote_equivocation::VoteEquivocationCf)?;
        if cf.exists_for_update(&key, OPERATION)? {
            return Ok(false);
        }
        cf.put(&key, evidence, OPERATION)?;
        Ok(true)
    }

    fn diagnostics_add_no_vote(&mut self, block_id: BlockId, reason: NoVoteReason) -> Result<(), StorageError> {
        const OPERATION: &str = "diagnostics_add_no_vote";
        if self.options.debugging_data {
            self.db().cf(DiagnosticsNoVoteCf)?.insert(
                &block_id,
                &DiagnosticsNoVoteData {
                    reason: reason.to_string().into_boxed_str(),
                },
                OPERATION,
            )?;
        }

        Ok(())
    }
}

impl<'a, TAddr> Deref for RocksDbStateStoreWriteTransaction<'a, TAddr> {
    type Target = RocksDbStateStoreReadTransaction<'a, TAddr>;

    fn deref(&self) -> &Self::Target {
        self.transaction.as_ref().expect("in deref: transaction is None")
    }
}

impl<TAddr> Drop for RocksDbStateStoreWriteTransaction<'_, TAddr> {
    fn drop(&mut self) {
        if self.transaction.is_some() {
            warn!(
                target: LOG_TARGET,
                "State store write transaction was not committed/rolled back. Rolling back"
            );
            // Take so that we mark this transaction as complete in the drop impl
            if let Err(err) = self
                .transaction
                .take()
                .expect("rollback: already committed")
                .into_parts()
                .0
                .rollback()
                .map_err(|source| RocksDbStorageError::RocksDbError {
                    source,
                    operation: "commit",
                })
            {
                error!(
                    target: LOG_TARGET,
                    "Failed to rollback state store write transaction: {}", err
                );
            }
        }
    }
}

fn recurse_subtree_depth_first_post_order<'a>(
    cf: &'a CfContext<Transaction<TransactionDB>, StateTreeCf>,
    shard: Shard,
    parent_key: NodeKey,
    children: Children,
) -> impl Iterator<Item = (Shard, NodeKey)> + 'a {
    const OPERATION: &str = "recurse_subtree";
    let parent_after_child = Some((shard, parent_key.clone()));

    children
        .into_iter()
        .flat_map(move |(nibble, child)| -> Box<dyn Iterator<Item = (Shard, NodeKey)>> {
            let child_key = match parent_key.gen_child_node_key(child.version, nibble) {
                Ok(child_key) => child_key,
                Err(e) => {
                    error!(target: LOG_TARGET, "Cannot derive the child of stale node ({shard}, {parent_key}) at {nibble:?}: {e}");
                    return Box::new(iter::empty());
                },
            };
            match child.node_type{
                NodeType::Leaf => {
                    Box::new(iter::once((shard, child_key)))
                }
                NodeType::Null => {
                    Box::new(iter::empty())
                }
                NodeType::Internal { .. } => {
                    let Some(child) = cf
                        .get(&(shard, child_key.clone()), OPERATION)
                        .optional()
                        .expect("db error in recurse_subtree")
                    else {
                        return Box::new(iter::empty());
                    };
                    let Node::Internal(x) = child else {
                        panic!("expected internal node in recurse_subtree for key ({shard}, {child_key}) but got {child:?}");
                    };

                    let children = x.into_children();
                    Box::new(recurse_subtree_depth_first_post_order(cf, shard, child_key, children))
                }

            }
        })
        // Emit the parent key after all children
        .chain(parent_after_child)
}

mod cleanup {
    use super::*;
    use crate::column_families::{
        certificates,
        certificates::{proposal::ProposalCertificateCf, timeout::TimeoutCertificateCf},
    };

    // Each function prunes at most `limit` records up to and including `up_to_epoch`, driven by an epoch-ordered
    // index whose entries it deletes along with their records, so the next call resumes where this one stopped.

    pub fn foreign_proposals_for_epoch(
        db: &DbWriteContext<'_>,
        up_to_epoch: Epoch,
        limit: usize,
    ) -> Result<usize, StorageError> {
        const OPERATION: &str = "cleanup::foreign_proposals_for_epoch";
        let up_to_epoch = up_to_epoch + Epoch(1); // Make it inclusive
        let entries = db.cf(foreign_proposal::ByEpochQuery)?.query_end_range_entries_limited(
            Ordering::Ascending,
            &up_to_epoch,
            limit,
        )?;

        let count = entries.len();
        for ((epoch, _), data) in entries {
            db.cf(ForeignProposalCf)?.delete(&data.block_id, OPERATION)?;
            db.cf(foreign_proposal::EpochIndex)?
                .delete(&(epoch, data.block_id), OPERATION)?;
            db.cf(foreign_proposal::UnconfirmedIndex)?
                .delete(&(epoch, data.block_id), OPERATION)?;
            if let Some(proposed_block_id) = data.proposed_in_block {
                db.cf(foreign_proposal::ProposedInBlockIndex)?
                    .delete(&(proposed_block_id, data.block_id), OPERATION)?;
            }
        }
        Ok(count)
    }

    pub fn blocks_for_epoch(db: &DbWriteContext<'_>, up_to_epoch: Epoch, limit: usize) -> Result<usize, StorageError> {
        const OPERATION: &str = "cleanup::blocks_for_epoch";
        let up_to_epoch = up_to_epoch + Epoch(1); // Make it inclusive
        let cf = db.cf(BlockCf)?;
        let committed_cf = db.cf(chain::CommittedParentChildChainIndex)?;
        let index_cf = db.cf(block::EpochHeightIndex)?;

        // Don't delete epoch 0 blocks (i.e the zero block)
        let keys =
            db.cf(block::ByEpochQuery)?
                .query_range_keys_limited(Ordering::Ascending, Epoch(1)..up_to_epoch, limit)?;

        let count = keys.len();
        for (epoch, height, block_id) in keys {
            cf.delete(&block_id, OPERATION)?;
            committed_cf.delete(&block_id, OPERATION)?;
            index_cf.delete(&(epoch, height, block_id), OPERATION)?;
        }
        Ok(count)
    }

    /// Prunes finalized transaction bookkeeping — the payload, finalized link, recorded executions
    /// and the epoch-index entry — for everything finalized up to and including `up_to_epoch`.
    ///
    /// Only bookkeeping is removed: a commit's effects and its receipt substate are synced state and
    /// remain, so committed ids stay refused via the receipt-existence check after their records are
    /// gone. The prune horizon matches block retention, so a transaction's payload outlives every
    /// retained block that references it.
    pub fn finalized_transactions_for_epoch(
        db: &DbWriteContext<'_>,
        up_to_epoch: Epoch,
        limit: usize,
    ) -> Result<usize, StorageError> {
        const OPERATION: &str = "cleanup::finalized_transactions_for_epoch";
        let up_to_epoch = up_to_epoch + Epoch(1); // Make it inclusive
        let tx_cf = db.cf(TransactionCf)?;
        let link_cf = db.cf(FinalizedTransactionLinkCf)?;
        let epoch_index_cf = db.cf(finalized_transaction::EpochIndex)?;
        let exec_cf = db.cf(BlockTransactionExecutionCf)?;
        let exec_query = db.cf(block_transaction_execution::ByTransactionIdQuery)?;
        let exec_index_cf = db.cf(block_transaction_execution::BlockIndex)?;

        let keys = db.cf(finalized_transaction::ByEpochQuery)?.query_range_keys_limited(
            Ordering::Ascending,
            Epoch::zero()..up_to_epoch,
            limit,
        )?;

        let count = keys.len();
        for (epoch, tx_id) in keys {
            tx_cf.delete(&tx_id, OPERATION)?;
            link_cf.delete(&tx_id, OPERATION)?;
            for (tx_id, block_id, epoch, height) in exec_query.query_prefix_range_keys(Ordering::default(), &tx_id)? {
                exec_cf.delete(&(tx_id, block_id, epoch, height), OPERATION)?;
                exec_index_cf.delete(&(block_id, tx_id, epoch, height), OPERATION)?;
            }
            epoch_index_cf.delete(&(epoch, tx_id), OPERATION)?;
        }
        Ok(count)
    }

    /// The log answers for heights of the epoch it belongs to, so it lives exactly as long as that
    /// epoch's blocks.
    pub fn validator_liveness_log_for_epoch(
        db: &DbWriteContext<'_>,
        up_to_epoch: Epoch,
        limit: usize,
    ) -> Result<usize, StorageError> {
        const OPERATION: &str = "cleanup::validator_liveness_log_for_epoch";
        let up_to_epoch = up_to_epoch + Epoch(1); // Make it inclusive
        let keys = db.cf(validator_liveness_log::ByEpochQuery)?.query_range_keys_limited(
            Ordering::Ascending,
            Epoch::zero()..up_to_epoch,
            limit,
        )?;

        let cf = db.cf(validator_liveness_log::ValidatorLivenessLogCf)?;
        for key in &keys {
            cf.delete(key, OPERATION)?;
        }
        Ok(keys.len())
    }

    /// Equivocation evidence is retained for as long as the blocks of the view it indicts, so an
    /// operator reading the record can still fetch the blocks it refers to.
    pub fn vote_equivocations_for_epoch(
        db: &DbWriteContext<'_>,
        up_to_epoch: Epoch,
        limit: usize,
    ) -> Result<usize, StorageError> {
        const OPERATION: &str = "cleanup::vote_equivocations_for_epoch";
        let up_to_epoch = up_to_epoch + Epoch(1); // Make it inclusive
        let keys = db.cf(vote_equivocation::ByEpochQuery)?.query_range_keys_limited(
            Ordering::Ascending,
            Epoch::zero()..up_to_epoch,
            limit,
        )?;

        let cf = db.cf(vote_equivocation::VoteEquivocationCf)?;
        for key in &keys {
            cf.delete(key, OPERATION)?;
        }
        Ok(keys.len())
    }

    pub fn proposal_certificates_for_epoch(
        db: &DbWriteContext<'_>,
        up_to_epoch: Epoch,
        limit: usize,
    ) -> Result<usize, StorageError> {
        const OPERATION: &str = "cleanup::proposal_certificates_for_epoch";
        let up_to_epoch = up_to_epoch + Epoch(1); // Make it inclusive
        let keys = db.cf(certificates::proposal::ByEpochQuery)?.query_range_keys_limited(
            Ordering::Ascending,
            Epoch(1)..up_to_epoch,
            limit,
        )?;

        let cf = db.cf(ProposalCertificateCf)?;
        for key in &keys {
            cf.delete(key, OPERATION)?;
        }
        Ok(keys.len())
    }

    pub fn timeout_certificates_for_epoch(
        db: &DbWriteContext<'_>,
        up_to_epoch: Epoch,
        limit: usize,
    ) -> Result<usize, StorageError> {
        const OPERATION: &str = "cleanup::timeout_certificates_for_epoch";
        let up_to_epoch = up_to_epoch + Epoch(1); // Make it inclusive
        let keys = db.cf(certificates::timeout::ByEpochQuery)?.query_range_keys_limited(
            Ordering::Ascending,
            Epoch(1)..up_to_epoch,
            limit,
        )?;

        let cf = db.cf(TimeoutCertificateCf)?;
        for key in &keys {
            cf.delete(key, OPERATION)?;
        }
        Ok(keys.len())
    }
}
