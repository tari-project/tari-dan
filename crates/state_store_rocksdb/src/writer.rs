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

use std::{cmp, collections::HashSet, iter, ops::Deref, time::Instant};

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
    optional::Optional,
    shard::Shard,
};
use tari_ootle_storage::{
    Ordering,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
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
        SubstateChange,
        SubstateCreated,
        SubstateDestroyed,
        SubstateLock,
        SubstatePledges,
        SubstateRecord,
        SubstateTransition,
        SubstateUpdateBatch,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionPoolStatusUpdate,
        TransactionRecord,
        ValidatorConsensusStats,
        ValidatorStatsUpdate,
    },
    time,
};
use tari_ootle_transaction::TransactionId;
use tari_state_tree::{Child, Nibble, Node, NodeKey, NodeType, StaleTreeNode, StateTreePayload, Version};

use crate::{
    cf_api::{CfContext, DbContext},
    codecs::{ByteColumn, DbEncoder, DefaultCodec},
    column_families::{
        block,
        block::BlockCf,
        block_diff,
        block_diff::{BlockDiffCf, BlockDiffKey},
        block_transaction_execution,
        block_transaction_execution::BlockTransactionExecutionCf,
        bookkeeping::{
            CommitBlock,
            CommitBlockCf,
            HighPcCf,
            HighTcCf,
            HighestSeenBlockCf,
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
        pending_state_tree_diff,
        pending_state_tree_diff::PendingStateTreeDiffCf,
        state_transition::{
            StateTransitionCf,
            StateTransitionModelDataV1,
            StateTransitionRecordData,
            StateTransitionType,
        },
        state_tree,
        state_tree::{StateTreeCf, StateTreeStaleNodesCf},
        state_tree_shard_versions::StateTreeShardVersionCf,
        substate,
        substate::{SubstateCf, SubstateHeadData},
        substate_locks,
        substate_locks::{SubstateLockKey, SubstateLockModel},
        transaction::TransactionCf,
        transaction_pool::TransactionPoolCf,
        transaction_pool_state_update,
        transaction_pool_state_update::{TransactionPoolStateUpdateCf, TransactionPoolStateUpdateData},
        validator_node_epoch_stats::ValidatorNodeEpochStatsCf,
    },
    error::RocksDbStorageError,
    options::DatabaseOptions,
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
    pub(crate) fn new(db: &'a TransactionDB, tx: Transaction<'a, TransactionDB>, options: &'a DatabaseOptions) -> Self {
        Self {
            db,
            // We have access to the inner transaction so we can use it to read/write
            transaction: Some(RocksDbStateStoreReadTransaction::new(db, ReadOnly::new(tx))),
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

        tx.into_rocksdb_transaction()
            .commit()
            .map_err(|source| RocksDbStorageError::RocksDbError {
                source,
                operation: "commit",
            })?;
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), StorageError> {
        // Take so that we mark this transaction as complete in the drop impl
        self.transaction
            .take()
            .expect("rollback: already committed")
            .into_rocksdb_transaction()
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
                    height: block.height(),
                    block_id: *block.id(),
                    parent_id: *block.parent(),
                },
                OPERATION,
            )?;
        }
        if let Some(value) = justify_qc_id {
            block.set_justify_qc(*value);
        }

        cf.put(block_id, &block, OPERATION)?;

        Ok(())
    }

    fn block_diffs_insert(&mut self, block_id: &BlockId, changes: &[SubstateChange]) -> Result<(), StorageError> {
        const OPERATION: &str = "block_diffs_insert";
        let cf = self.db().cf(BlockDiffCf)?;
        let index_cf = self.db().cf(block_diff::SubstateIdIndex)?;

        assert!(
            changes.len() <= u32::MAX as usize,
            "BlockDiffs cannot exceed u32::MAX (>4 billion) changes, got {}",
            changes.len()
        );
        for (seq, change) in changes.iter().enumerate() {
            let key = BlockDiffKey {
                block_id: *block_id,
                sequence: seq as u32,
                substate_id: change.versioned_substate_id().substate_id().clone(),
                version: change.versioned_substate_id().version(),
                is_up: change.is_up(),
            };
            cf.put(&key, change, OPERATION)?;
            // Note: the key is encoded with substate id first
            index_cf.put(&key, &(), OPERATION)?;
        }

        Ok(())
    }

    fn block_diffs_remove(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        const OPERATION: &str = "block_diffs_remove";
        let cf = self.db().cf(BlockDiffCf)?;
        let index_cf = self.db().cf(block_diff::SubstateIdIndex)?;
        let query = self.db().cf(block_diff::ByBlockIdQuery)?;
        let iter = query.query_prefix_range_key_iterator(Ordering::Ascending, block_id);
        for result in iter {
            let key = result?;
            cf.delete(&key, OPERATION)?;
            index_cf.delete(&key, OPERATION)?;
        }

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
        let proposed_iter = cf.query_prefix_range_key_iterator(Ordering::default(), proposed_in_block);

        for result in proposed_iter {
            let (proposed_in_block, fp_id) = result?;
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

        let iter = exec_query.query_prefix_range_key_iterator(Ordering::default(), tx_id);
        for result in iter {
            let (tx_id, block_id, height) = result?;
            exec_cf.delete(&(tx_id, block_id, height), OPERATION)?;
            exec_index_cf.delete(&(block_id, tx_id, height), OPERATION)?;
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
                transaction_execution.block_height(),
            ),
            transaction_execution,
            OPERATION,
        )?;

        self.db().cf(block_transaction_execution::BlockIndex)?.put(
            &(
                *transaction_execution.block_id(),
                *transaction_execution.transaction_id(),
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

        let iter = query.query_prefix_range_key_iterator(Ordering::default(), block_id);
        for result in iter {
            let key = result?;
            index_cf.delete(&key, OPERATION)?;
            let (block_id, tx_id, height) = key;
            cf.delete(&(tx_id, block_id, height), OPERATION)?;
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
        let iter = block_query.query_prefix_range_key_iterator(Ordering::default(), lock_block.block_id());
        for result in iter {
            let (_, tx_id, locked_height) = result?;
            let tx_iter = tx_query.query_prefix_range_key_iterator(Ordering::default(), &tx_id);
            for result in tx_iter {
                let (tx_id, block_id, height) = result?;
                // Don't remove for this block or any later blocks (higher height)
                if height > locked_height {
                    trace!(
                        target: LOG_TARGET,
                        "Skip deleting transaction execution for transaction {} in block {} ({} > {})",
                        tx_id,
                        block_id,
                        height,
                        locked_height
                    );
                    continue;
                }
                if block_id == *lock_block.block_id() {
                    continue;
                }
                debug!(
                    target: LOG_TARGET,
                    "Deleting transaction execution for transaction {} in block {} ({} <= {})",
                    tx_id,
                    block_id,
                    height,
                    locked_height
                );
                cf.delete(&(tx_id, block_id, height), OPERATION)?;
                index_cf.delete(&(block_id, tx_id, height), OPERATION)?;
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

        let iter = by_block_query.query_prefix_range_iterator(Ordering::Ascending, block.block_id());

        let updates_cf = self.db().cf(TransactionPoolStateUpdateCf)?;
        let pool_cf = self.db().cf(TransactionPoolCf)?;
        for result in iter {
            let (key, update) = result?;
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

        let iter = by_block_query.query_prefix_range_key_iterator(Ordering::Ascending, block_id);

        let updates_cf = self.db().cf(TransactionPoolStateUpdateCf)?;
        for result in iter {
            let key = result?;
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
        for tx_id in missing_transaction_ids {
            cf.put(&(*tx_id, *park_block_id), &(), OPERATION)?;
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
        let iter = query.query_prefix_range_key_iterator(Ordering::default(), transaction_id);

        // Remove the transaction ids from the missing list
        let mut block_ids = HashSet::new();
        for result in iter {
            let (transaction_id, block_id) = result?;
            block_ids.insert(block_id);
            missing_cf.delete(&(transaction_id, block_id), OPERATION)?;
        }

        if block_ids.is_empty() {
            return Ok(vec![]);
        }

        // Check if there are any remaining for this block - TODO: consider optimising, loops through all entries
        let iter = missing_cf.key_iterator(Ordering::default(), OPERATION);
        for result in iter {
            let (_, block_id) = result?;
            if block_ids.contains(&block_id) {
                block_ids.remove(&block_id);
            }
        }

        // If ALL of the blocks still have missing transactions, exit early
        if block_ids.is_empty() {
            return Ok(vec![]);
        }

        // Unpark (fetch and delete) the blocks
        let blocks = cf.multi_get(&block_ids, OPERATION)?;
        for id in &block_ids {
            cf.delete(id, OPERATION)?;
        }

        Ok(blocks)
    }

    fn substate_locks_insert_all<'a, I: IntoIterator<Item = (&'a SubstateId, &'a Vec<SubstateLock>)>>(
        &mut self,
        block: &LeafBlock,
        locks: I,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "substate_locks_insert_all";

        let cf = self.db().cf(SubstateLockModel)?;
        let index_cf = self.db().cf(substate_locks::BlockIdIndex)?;
        let substate_index_cf = self.db().cf(substate_locks::SubstateIdIndex)?;
        let head_index = self.db().cf(substate_locks::HeadIndex)?;
        for (substate_id, locks) in locks {
            let mut last_key = None;
            for lock in locks {
                let key = SubstateLockKey {
                    block_id: *block.block_id(),
                    block_height: block.height(),
                    substate_id: substate_id.clone(),
                    transaction_id: *lock.transaction_id(),
                };
                cf.put(&key, lock, OPERATION)?;
                index_cf.put(&key, &(), OPERATION)?;
                substate_index_cf.put(&key, &lock.lock_type(), OPERATION)?;
                last_key = Some(key);
            }
            if let Some(key) = last_key {
                head_index.put(substate_id, &key, OPERATION)?;
            }
        }

        Ok(())
    }

    fn substate_locks_remove_many_for_transactions<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "substate_locks_remove_many_for_transactions";
        // check the peekable iterator to save an OP.
        let mut transaction_ids = transaction_ids.into_iter().peekable();
        if transaction_ids.peek().is_none() {
            return Ok(());
        }

        let cf = self.db().cf(SubstateLockModel)?;
        let query_cf = self.db().cf(substate_locks::ByTransactionIdQuery)?;
        let substate_index_cf = self.db().cf(substate_locks::SubstateIdIndex)?;
        let head_index_cf = self.db().cf(substate_locks::HeadIndex)?;
        let index_cf = self.db().cf(substate_locks::BlockIdIndex)?;
        for tx_id in transaction_ids {
            let iter = query_cf.query_prefix_range_key_iterator(Ordering::default(), tx_id);
            for result in iter {
                let key = result?;
                trace!(
                    target: LOG_TARGET,
                    "Removing substate locks {key}",
                );
                cf.delete(&key, OPERATION)?;
                index_cf.delete(&key, OPERATION)?;
                substate_index_cf.delete(&key, OPERATION)?;
                // TODO: this could leave the head index in an inconsistent state - I suspect we should implement locks
                // in-memory instead of in persistence perhaps (or not) persisting the entire lock state asynchronously
                // as blocks are processed (to account for node restarts)
                if let Some(head_key) = head_index_cf.get(&key.substate_id, OPERATION).optional()? &&
                    head_key.transaction_id == key.transaction_id
                {
                    head_index_cf.delete(&key.substate_id, OPERATION)?;
                }
            }
        }

        Ok(())
    }

    fn substate_locks_remove_any_by_block_id(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        const OPERATION: &str = "substate_locks_remove_any_by_block_id";

        let cf = self.db().cf(SubstateLockModel)?;
        let index_cf = self.db().cf(substate_locks::BlockIdIndex)?;
        let substate_index_cf = self.db().cf(substate_locks::SubstateIdIndex)?;
        let head_index_cf = self.db().cf(substate_locks::HeadIndex)?;
        let query_cf = self.db().cf(substate_locks::ByBlockIdQuery)?;
        let iter = query_cf.query_prefix_range_key_iterator(Ordering::Ascending, block_id);
        for result in iter {
            let key = result?;
            cf.delete(&key, OPERATION)?;
            index_cf.delete(&key, OPERATION)?;
            substate_index_cf.delete(&key, OPERATION)?;
            if let Some(head_key) = head_index_cf.get(&key.substate_id, OPERATION).optional()? &&
                head_key.block_id == key.block_id
            {
                head_index_cf.delete(&key.substate_id, OPERATION)?;
            }
        }

        Ok(())
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

                            let mut substate = cf.get(&address, OPERATION)?;
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

    fn substates_prune_downed_values(&mut self, epoch: Epoch) -> Result<usize, StorageError> {
        const OPERATION: &str = "substates_prune_downed_values";
        let db = self.db();
        let unpruned_query = db.cf(substate::UnprunedDownedValuesEpochQuery)?;
        let unpruned_index = db.cf(substate::UnprunedDownedValuesIndex)?;
        let iter = unpruned_query.query_prefix_range_iterator(Ordering::Ascending, &epoch);
        let substates_cf = db.cf(SubstateCf)?;
        let mut count = 0usize;
        for result in iter {
            let (key, addresses) = result?;

            // TODO(perf): consider storing the actual values in a separate column family to avoid get/set
            for substate_addr in addresses {
                let mut substate = substates_cf.get(&substate_addr, OPERATION)?;
                substate.clear_substate_value();
                substates_cf.put(&substate_addr, &substate, OPERATION)?;
                count += 1;
            }
            unpruned_index.delete(&key, OPERATION)?;
        }
        info!(
            target: LOG_TARGET,
            "🗑️ Pruned {count} downed substates for epoch {epoch} from unpruned values index"
        );

        Ok(count)
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
            let iter = query.query_prefix_range_key_iterator(Ordering::default(), transaction_id);
            for result in iter {
                let key = result?;
                cf.delete(&key, OPERATION)?;
            }
        }

        Ok(())
    }

    fn pending_state_tree_diffs_insert(
        &mut self,
        block_id: BlockId,
        shard: Shard,
        diff: &PendingShardStateTreeDiff,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "pending_state_tree_diffs_insert";
        trace!(
            target: LOG_TARGET,
            "{OPERATION}: shard {} block {} (v{}, new={}, stale={})", shard, block_id,
            diff.version,diff.diff.new_nodes.len(),diff.diff.stale_tree_nodes.len()
        );
        self.db()
            .cf(PendingStateTreeDiffCf)?
            .put(&(block_id, shard), diff, OPERATION)?;
        Ok(())
    }

    fn pending_state_tree_diffs_remove_by_block(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        const OPERATION: &str = "pending_state_tree_diffs_remove_by_block";
        let cf = self.db().cf(PendingStateTreeDiffCf)?;
        let query = self.db().cf(pending_state_tree_diff::ByBlockIdQuery)?;
        let iter = query.query_prefix_range_key_iterator(Ordering::Ascending, block_id);

        for result in iter {
            let key = result?;
            cf.delete(&key, OPERATION)?;
        }

        Ok(())
    }

    fn pending_state_tree_diffs_remove_and_return_by_block(
        &mut self,
        block_id: &BlockId,
    ) -> Result<IndexMap<Shard, Vec<PendingShardStateTreeDiff>>, StorageError> {
        const OPERATION: &str = "pending_state_tree_diffs_remove_and_return_by_block";
        let cf = self.db().cf(PendingStateTreeDiffCf)?;
        let query = self.db().cf(pending_state_tree_diff::ByBlockIdQuery)?;
        let iter = query.query_prefix_range_iterator(Ordering::Ascending, block_id);

        let mut diffs = IndexMap::new();
        for result in iter {
            let (key, diff) = result?;
            let (_, shard) = &key;
            diffs.entry(*shard).or_insert_with(Vec::new).push(diff);
            cf.delete(&key, OPERATION)?;
        }

        Ok(diffs)
    }

    fn state_tree_nodes_batch_insert(
        &mut self,
        shard: Shard,
        nodes: Vec<(NodeKey, Node<StateTreePayload>)>,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "state_tree_nodes_insert";
        let cf = self.db().cf(StateTreeCf)?;
        for (key, node) in nodes {
            cf.put(&(shard, key), &node, OPERATION)?;
        }
        Ok(())
    }

    fn state_tree_nodes_record_stale_tree_nodes(
        &mut self,
        shard: Shard,
        version: Version,
        nodes: Vec<StaleTreeNode>,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "state_tree_nodes_record_stale_tree_nodes";

        self.db()
            .cf(StateTreeStaleNodesCf)?
            .put(&(shard, version), &nodes, OPERATION)?;

        Ok(())
    }

    fn state_tree_nodes_clear_stale(&mut self, num_preshards: NumPreshards) -> Result<usize, StorageError> {
        const OPERATION: &str = "state_tree_nodes_clear_all_stale";
        /// We buffer deletes to ensure that we delete entire subtrees at once. The number of buffered deletes may
        /// exceed this threshold when flushed, due to whole subtrees being added.
        const DELETE_BUFFER_FLUSH_THRESHOLD: usize = 100_000;

        let cf = self.db().cf(StateTreeCf)?;
        let versions_cf = self.db().cf(StateTreeShardVersionCf)?;
        let stale_cf = self.db().cf(state_tree::ByStateTreeStaleShardQuery)?;

        let mut delete_buffer = Vec::new();
        let mut total_num_deleted = 0usize;
        let shards = iter::once(Shard::global()).chain(ShardGroup::all_shards(num_preshards).shard_iter());
        for shard in shards {
            let timer = Instant::now();
            let mut num_deleted = 0;
            let stale_iter = stale_cf.query_prefix_range_iterator(Ordering::Ascending, &shard);
            let max_version = versions_cf.get(&shard, OPERATION).optional()?.unwrap_or(0);
            let Some(to_version) = max_version.checked_sub(self.options.state_history_length) else {
                trace!(target: LOG_TARGET, "Shard {shard} is at version {max_version}, skipping stale node deletion due to history length {}", self.options.state_history_length);
                continue;
            };
            for result in stale_iter {
                let ((shard, version), nodes) = result?;
                // Only delete up to history length back from the max version
                if version > to_version {
                    break;
                }

                for node in nodes {
                    // Deletes are buffered to ensure that we delete entire subtrees at once.
                    if delete_buffer.len() >= DELETE_BUFFER_FLUSH_THRESHOLD {
                        debug!(target: LOG_TARGET, "Deleting {} stale nodes from shard {}", delete_buffer.len(), shard);
                        for key in &*delete_buffer {
                            cf.delete(key, OPERATION)?;
                        }
                        num_deleted += delete_buffer.len();
                        delete_buffer.clear();
                    }

                    match node {
                        StaleTreeNode::Node(key) => {
                            trace!(target: LOG_TARGET, "Deleting stale node {key} from shard {shard}", );
                            // Lazy allocation
                            if delete_buffer.capacity() == 0 {
                                delete_buffer.reserve_exact(DELETE_BUFFER_FLUSH_THRESHOLD); // ~3.3 MB for 100k entries (excl. nibble path vec)
                            }
                            delete_buffer.push((shard, key));
                        },
                        StaleTreeNode::Subtree(parent_key) => {
                            trace!(target: LOG_TARGET, "Deleting stale substree {parent_key} from shard {shard}", );
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
                                    // Subtree is a single leaf node
                                    trace!(target: LOG_TARGET, "Deleting stale leaf node {parent_key} from shard {shard}", );
                                    // Lazy allocation
                                    if delete_buffer.capacity() == 0 {
                                        delete_buffer.reserve_exact(DELETE_BUFFER_FLUSH_THRESHOLD); // ~3.3 MB for 100k entries (excl. nibble path vec)
                                    }
                                    delete_buffer.push((shard, parent_key));
                                },
                                Node::Null => {},
                            }
                        },
                    }
                }

                if !delete_buffer.is_empty() {
                    debug!(target: LOG_TARGET, "Deleting last {} stale nodes from shard {}", delete_buffer.len(), shard);
                    for key in &delete_buffer {
                        cf.delete(key, OPERATION)?;
                    }
                    num_deleted += delete_buffer.len();
                    delete_buffer.clear();
                }

                // Finally delete the stale node record
                self.db()
                    .cf(StateTreeStaleNodesCf)?
                    .delete(&(shard, version), OPERATION)?;
            }

            if num_deleted > 0 {
                total_num_deleted += num_deleted;
                debug!(
                    target: LOG_TARGET,
                    "Deleted {} stale nodes in shard {} in {:.2?} to version {}",
                    num_deleted, shard, timer.elapsed(), to_version
                );
            }
        }

        Ok(total_num_deleted)
    }

    fn state_tree_shard_versions_set(&mut self, shard: Shard, version: Version) -> Result<(), StorageError> {
        const OPERATION: &str = "state_tree_shard_versions_set";

        self.db()
            .cf(StateTreeShardVersionCf)?
            .put(&shard, &version, OPERATION)?;

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
            let iter = query.query_prefix_range_key_iterator(Ordering::Ascending, tx_id);
            for result in iter {
                let key = result?;
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

        let iter = query_cf.query_prefix_range_key_iterator(Ordering::Ascending, block_id);
        for result in iter {
            let key = result?;
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
        updates: I,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "validator_epoch_stats_updates";

        let cf = self.db().cf(ValidatorNodeEpochStatsCf)?;
        for update in updates {
            let existing = cf.get(&(epoch, *update.public_key()), OPERATION).optional()?;

            match existing {
                Some(mut existing) => match update.missed_proposal_change() {
                    Some(0) => {
                        existing.participation_shares += update.participation_shares_increment();
                        existing.missed_proposals = 0;
                        cf.put(&(epoch, *update.public_key()), &existing, OPERATION)?;
                    },
                    Some(n) => {
                        // NOTE: n can be negative
                        existing.participation_shares += update.participation_shares_increment();
                        existing.missed_proposals = cmp::max(existing.missed_proposals as i64 + n, 0) as u64;
                        cf.put(&(epoch, *update.public_key()), &existing, OPERATION)?;
                    },
                    None => {},
                },
                None => {
                    let leader_failure_inc = update.missed_proposal_change().map_or(0i64, |set| set.max(0));
                    let rec = ValidatorConsensusStats {
                        participation_shares: update.participation_shares_increment(),
                        missed_proposals: leader_failure_inc as u64,
                    };
                    cf.put(&(epoch, *update.public_key()), &rec, OPERATION)?;
                },
            }
        }

        Ok(())
    }

    fn epoch_cleanup(&mut self, epoch: Epoch) -> Result<(), StorageError> {
        let Some(prune_epoch) = epoch.checked_sub(self.options.epoch_history_length) else {
            return Ok(());
        };

        // TODO: this assumes that cleanup is run every epoch - if not, some substates will not be pruned
        let n = self.substates_prune_downed_values(prune_epoch)?;
        if n > 0 {
            info!(
                target: LOG_TARGET,
                "🗑️ Pruned {n} downed substates for epoch {} during epoch cleanup for epoch {}",
                prune_epoch, epoch
            );
        }

        let db = self.db();
        cleanup::cleanup_blocks_for_epoch(&db, prune_epoch)?;
        cleanup::cleanup_qcs_for_epoch(&db, prune_epoch)?;
        cleanup::foreign_proposals_for_epoch(&db, prune_epoch)?;
        if self.options.prune_transaction_history {
            cleanup::cleanup_finalized_transactions_for_epoch(&db, prune_epoch)?;
        }

        Ok(())
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
                .into_rocksdb_transaction()
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
    children: IndexMap<Nibble, Child>,
) -> impl Iterator<Item = (Shard, NodeKey)> + 'a {
    const OPERATION: &str = "recurse_subtree";
    let parent_after_child = Some((shard, parent_key.clone()));

    children
        .into_iter()
        .flat_map(move |(nibble, child)| -> Box<dyn Iterator<Item = (Shard, NodeKey)>> {
            let child_key = parent_key.gen_child_node_key(child.version, nibble);
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

    pub fn foreign_proposals_for_epoch(db: &DbWriteContext<'_>, up_to_epoch: Epoch) -> Result<(), StorageError> {
        const OPERATION: &str = "cleanup::foreign_proposals_for_epoch";
        let up_to_epoch = up_to_epoch + Epoch(1); // Make it inclusive
        let cf = db.cf(foreign_proposal::ByEpochQuery)?;
        let iter = cf.query_end_range_iterator(Ordering::Ascending, &up_to_epoch);

        let mut count = 0;
        for result in iter {
            let ((epoch, _), data) = result?;
            db.cf(ForeignProposalCf)?.delete(&data.block_id, OPERATION)?;
            db.cf(foreign_proposal::EpochIndex)?
                .delete(&(epoch, data.block_id), OPERATION)?;
            db.cf(foreign_proposal::UnconfirmedIndex)?
                .delete(&(epoch, data.block_id), OPERATION)?;
            if let Some(proposed_block_id) = data.proposed_in_block {
                db.cf(foreign_proposal::ProposedInBlockIndex)?
                    .delete(&(proposed_block_id, data.block_id), OPERATION)?;
            }
            count += 1;
        }
        info!(
            target: LOG_TARGET,
            "Cleaned up {} foreign proposals for epoch ..{}",
            count,
            up_to_epoch
        );
        Ok(())
    }

    pub fn cleanup_blocks_for_epoch(db: &DbWriteContext<'_>, up_to_epoch: Epoch) -> Result<(), StorageError> {
        const OPERATION: &str = "cleanup::cleanup_blocks_for_epoch";
        let up_to_epoch = up_to_epoch + Epoch(1); // Make it inclusive
        let cf = db.cf(BlockCf)?;
        let committed_cf = db.cf(chain::CommittedParentChildChainIndex)?;
        let query = db.cf(block::ByEpochQuery)?;
        let index_cf = db.cf(block::EpochHeightIndex)?;

        // Don't delete epoch 0 blocks (i.e the zero block)
        let iter = query.query_range_key_iterator(Ordering::Ascending, Epoch(1)..up_to_epoch);

        let mut count = 0usize;
        for result in iter {
            let (epoch, height, block_id) = result?;
            cf.delete(&block_id, OPERATION)?;
            committed_cf.delete(&block_id, OPERATION)?;
            index_cf.delete(&(epoch, height, block_id), OPERATION)?;
            count += 1;
        }

        info!(
            target: LOG_TARGET,
            "Cleaned up {} blocks for ..{}",
            count,
            up_to_epoch
        );

        Ok(())
    }

    /// Prunes finalized transaction bookkeeping — the payload, finalized link, recorded executions
    /// and the epoch-index entry — for everything finalized up to and including `up_to_epoch`.
    ///
    /// Only bookkeeping is removed: a commit's effects and its receipt substate are synced state and
    /// remain, so committed ids stay refused via the receipt-existence check after their records are
    /// gone. The prune horizon matches block retention, so a transaction's payload outlives every
    /// retained block that references it.
    pub fn cleanup_finalized_transactions_for_epoch(
        db: &DbWriteContext<'_>,
        up_to_epoch: Epoch,
    ) -> Result<(), StorageError> {
        const OPERATION: &str = "cleanup::cleanup_finalized_transactions_for_epoch";
        let up_to_epoch = up_to_epoch + Epoch(1); // Make it inclusive
        let tx_cf = db.cf(TransactionCf)?;
        let link_cf = db.cf(FinalizedTransactionLinkCf)?;
        let epoch_index_cf = db.cf(finalized_transaction::EpochIndex)?;
        let exec_cf = db.cf(BlockTransactionExecutionCf)?;
        let exec_query = db.cf(block_transaction_execution::ByTransactionIdQuery)?;
        let exec_index_cf = db.cf(block_transaction_execution::BlockIndex)?;

        let query = db.cf(finalized_transaction::ByEpochQuery)?;
        let iter = query.query_range_key_iterator(Ordering::Ascending, Epoch::zero()..up_to_epoch);

        let mut count = 0usize;
        for result in iter {
            let (epoch, tx_id) = result?;
            tx_cf.delete(&tx_id, OPERATION)?;
            link_cf.delete(&tx_id, OPERATION)?;
            let exec_iter = exec_query.query_prefix_range_key_iterator(Ordering::default(), &tx_id);
            for result in exec_iter {
                let (tx_id, block_id, height) = result?;
                exec_cf.delete(&(tx_id, block_id, height), OPERATION)?;
                exec_index_cf.delete(&(block_id, tx_id, height), OPERATION)?;
            }
            epoch_index_cf.delete(&(epoch, tx_id), OPERATION)?;
            count += 1;
        }

        info!(
            target: LOG_TARGET,
            "Cleaned up {} finalized transactions for ..{}",
            count,
            up_to_epoch
        );

        Ok(())
    }

    pub fn cleanup_qcs_for_epoch(db: &DbWriteContext<'_>, up_to_epoch: Epoch) -> Result<(), StorageError> {
        const OPERATION: &str = "cleanup::cleanup_qcs_for_epoch";
        let up_to_epoch = up_to_epoch + Epoch(1); // Make it inclusive
        let cf = db.cf(ProposalCertificateCf)?;
        let query = db.cf(certificates::proposal::ByEpochQuery)?;
        let iter = query.query_range_key_iterator(Ordering::Ascending, Epoch(1)..up_to_epoch);

        let mut count = 0usize;
        for result in iter {
            let key = result?;
            cf.delete(&key, OPERATION)?;
            count += 1;
        }

        info!(
            target: LOG_TARGET,
            "Cleaned up {} proposal certificates for ..{}",
            count,
            up_to_epoch
        );

        let cf = db.cf(TimeoutCertificateCf)?;
        let query = db.cf(certificates::timeout::ByEpochQuery)?;
        let iter = query.query_range_key_iterator(Ordering::Ascending, Epoch(1)..up_to_epoch);

        let mut count = 0usize;
        for result in iter {
            let key = result?;
            cf.delete(&key, OPERATION)?;
            count += 1;
        }

        info!(
            target: LOG_TARGET,
            "Cleaned up {} timeout certificates for ..={}",
            count,
            up_to_epoch
        );

        Ok(())
    }
}
