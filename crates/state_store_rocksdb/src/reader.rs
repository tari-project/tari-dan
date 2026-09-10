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

use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
};

use log::*;
use rocksdb::{Transaction, TransactionDB};
use serde::{Serialize, de::DeserializeOwned};
use tari_consensus_types::{
    BlockId,
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
    ShardGroup,
    ShardStateVersions,
    StateVersion,
    SubstateAddress,
    ToSubstateAddress,
    VersionedSubstateIdRef,
    displayable::Displayable,
    optional::Optional,
    shard::Shard,
};
use tari_ootle_storage::{
    Ordering,
    StateStoreReadTransaction,
    StorageError,
    consensus_models::{
        Block,
        BlockDiff,
        BlockTransactionExecution,
        EpochCheckpoint,
        ForeignProposalRecord,
        LockedSubstateValue,
        PendingShardStateTreeDiff,
        StateVersionTransitions,
        SubstateChange,
        SubstateCreate,
        SubstateData,
        SubstateDestroy,
        SubstateLock,
        SubstatePledges,
        SubstateRecord,
        SubstateUpdateProof,
        SubstateValueFilterFlags,
        SubstateValueOrHash,
        TransactionExecution,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionRecord,
        ValidatorConsensusStats,
    },
    time::PrimitiveDateTime,
};
use tari_ootle_transaction::TransactionId;
use tari_state_tree::{Node, NodeKey, StateTreePayload, Version};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{
    cf_api::DbContext,
    codecs::DbEncoder,
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
        epoch_checkpoint,
        epoch_checkpoint::EpochCheckpointCf,
        finalized_transaction::FinalizedTransactionLinkCf,
        foreign_parked_blocks::ForeignParkedBlockCf,
        foreign_proposal,
        foreign_proposal::ForeignProposalCf,
        foreign_substate_pledge,
        foreign_substate_pledge::ForeignSubstatePledgeCf,
        lock_conflict,
        parked_block,
        pending_state_tree_diff,
        state_transition,
        state_transition::StateTransitionType,
        state_tree,
        state_tree::StateTreeCf,
        state_tree_shard_versions,
        state_tree_shard_versions::StateTreeShardVersionCf,
        substate,
        substate::SubstateCf,
        substate_locks,
        substate_locks::SubstateLockModel,
        transaction::TransactionCf,
        transaction_pool::TransactionPoolCf,
        transaction_pool_state_update,
        validator_node_epoch_stats::ValidatorNodeEpochStatsCf,
    },
    error::RocksDbStorageError,
    read_only::ReadOnly,
    state_tree_iterator::LatestSubstateStateTreeIterator,
    traits::{Cf, RocksReader},
};

const LOG_TARGET: &str = "tari::ootle::storage::state_store_rocksdb::reader";

pub(crate) type ReadOnlyTransaction<'a> = ReadOnly<Transaction<'a, TransactionDB>>;

pub struct RocksDbStateStoreReadTransaction<'a, TAddr, R = ReadOnlyTransaction<'a>> {
    tx: R,
    db: &'a TransactionDB,
    _addr: PhantomData<TAddr>,
    // A read view must not be held open across an `.await` or *moved* to another thread: a live
    // snapshot pins SST files (space amplification), and reads are meant to be short and scoped.
    // This marker makes the type `!Send`, turning "held across an await in a Send future" into a
    // compile error rather than a documented discipline. It also drops the auto `Sync` impl, which is
    // restored below so `&ReadView` can still be shared for parallel reads on one snapshot.
    // See CONTEXT.md (read view).
    _not_send: PhantomData<*const ()>,
}

// SAFETY: a read view exposes only `&self` read operations. When the backing reader `R` is `Sync`,
// the snapshot and `&TransactionDB` it holds are thread-safe to share, so `&ReadView` may be shared
// across threads (e.g. `std::thread::scope`/rayon) for parallel reads on one consistent snapshot. The
// type stays `!Send` (the `_not_send` marker), so a view still cannot be moved across a thread
// boundary or held across an `.await` in a `Send` future. `PhantomData<TAddr>` holds no value, so
// `TAddr: Sync` is not required.
unsafe impl<TAddr, R: Sync> Sync for RocksDbStateStoreReadTransaction<'_, TAddr, R> {}

impl<'a, TAddr, R> RocksDbStateStoreReadTransaction<'a, TAddr, R> {
    pub(crate) fn new(db: &'a TransactionDB, tx: R) -> Self {
        Self {
            tx,
            db,
            _addr: PhantomData,
            _not_send: PhantomData,
        }
    }

    pub fn db(&self) -> DbContext<'_, R> {
        DbContext::new(self.db, &self.tx)
    }
}

// `rocksdb_transaction`/`into_rocksdb_transaction` only exist on the transaction-backed instantiation
// (the writer uses them for commit/rollback/drop). A snapshot-backed read view has no transaction.
impl<'a, TAddr> RocksDbStateStoreReadTransaction<'a, TAddr, ReadOnlyTransaction<'a>> {
    pub(crate) fn rocksdb_transaction(&self) -> &Transaction<'a, TransactionDB> {
        &self.tx.inner
    }

    pub(crate) fn into_rocksdb_transaction(self) -> Transaction<'a, TransactionDB> {
        self.tx.inner
    }
}

impl<'a, TAddr: NodeAddressable + Serialize + DeserializeOwned + 'a, R: RocksReader>
    RocksDbStateStoreReadTransaction<'a, TAddr, R>
{
    /// Returns the blocks until the end_block (inclusive). NOTE: there is no specific order in the returned blocks
    /// (HashSet) so this should only be used to determine ex/inclusion in the set. The end_block should be a block
    /// in the pending chain if not an empty list is returned.
    fn get_pending_chain_until(&self, end_block: &BlockId) -> Result<HashSet<BlockId>, RocksDbStorageError> {
        const OPERATION: &str = "get_pending_chain_until";
        trace!(target: LOG_TARGET, "{OPERATION}: end: {end_block}");

        let chain_cf = self.db().cf(chain::PendingChainIndex)?;
        if !chain_cf.exists(end_block, OPERATION)? {
            return Ok(HashSet::new());
        }

        let mut block_ids = HashSet::new();
        block_ids.insert(*end_block);
        let mut block_id = *end_block;

        while let Some(parent_id) = chain_cf.get(&block_id, OPERATION).optional()? {
            block_ids.insert(parent_id);
            block_id = parent_id;
            if parent_id == BlockId::zero() {
                break;
            }
        }

        Ok(block_ids)
    }

    /// Returns the blocks until the end_block (inclusive) ordered from the end_block to the commit block (height
    /// descending).
    pub(super) fn get_pending_chain_ordered(&self, end_block: &BlockId) -> Result<Vec<BlockId>, RocksDbStorageError> {
        // TODO: only difference between get_pending_chain_until is that this returns a Vec - worth DRYing up
        const OPERATION: &str = "get_pending_chain_ordered";

        let chain_cf = self.db().cf(chain::PendingChainIndex)?;
        if !chain_cf.exists(end_block, OPERATION)? {
            trace!(
                target: LOG_TARGET,
                "{OPERATION}: end block {end_block} not in pending chain",
            );
            return Ok(Vec::new());
        }

        let mut block_ids = Vec::new();
        block_ids.push(*end_block);
        let mut block_id = *end_block;
        trace!(
            target: LOG_TARGET,
            "{OPERATION}: end block {end_block} is in pending chain",
        );

        let commit_block = self.get_commit_block()?;

        while let Some(parent_id) = chain_cf.get(&block_id, OPERATION).optional()? {
            trace!(
                target: LOG_TARGET,
                "{OPERATION}: {block_id} parent_id: {parent_id}",
            );

            // The commit block is the parent of the final block, don't include it
            if parent_id == BlockId::zero() || parent_id == commit_block.block_id {
                break;
            }

            block_ids.push(parent_id);
            block_id = parent_id;
        }

        trace!(
            target: LOG_TARGET,
            "{OPERATION}: block_ids.len(): {}",
            block_ids.len()
        );

        Ok(block_ids)
    }

    fn get_pending_chain_with_commands_between(
        &self,
        end_block: &BlockId,
    ) -> Result<HashSet<BlockId>, RocksDbStorageError> {
        // TODO: This is just an optimisation that returns less blocks, for now we just return all pending chain blocks
        self.get_pending_chain_until(end_block)
    }

    /// Used in tests, therefore not used in consensus and not part of the trait
    pub fn transactions_count(&self) -> Result<u64, RocksDbStorageError> {
        const OPERATION: &str = "transactions_count";
        self.db().cf(TransactionCf)?.count(OPERATION).map(|c| c as u64)
    }

    pub fn substates_count(&self) -> Result<u64, RocksDbStorageError> {
        const OPERATION: &str = "substates_count";
        self.db().cf(SubstateCf)?.count(OPERATION).map(|c| c as u64)
    }

    fn get_current_locked_block(&self) -> Result<LockedBlock, StorageError> {
        let cf = self.db().cf(LockedBlockCf)?;
        let value = cf.get_by_default_key("get_current_locked_block")?;
        Ok(value)
    }

    pub fn blocks_get_parent_chain(&self, start_block_id: &BlockId, limit: usize) -> Result<Vec<Block>, StorageError> {
        // Only used for JSON-RPC - not optimised
        const OPERATION: &str = "blocks_get_parent_chain";

        let Some(locked_block) = self.get_current_locked_block().optional()? else {
            return Err(StorageError::QueryError {
                reason: format!("{OPERATION}: No locked block found"),
            });
        };

        let cf = self.db().cf(BlockCf)?;

        let query = self.db().cf(block::ByEpochQuery)?;
        let iter = query.query_prefix_range_key_iterator(Ordering::Descending, &locked_block.epoch());

        let mut blocks = vec![];
        let mut is_found = false;
        for result in iter {
            let (_, _, block_id) = result?;
            // "Scan" for the start block
            if !is_found {
                if block_id == *start_block_id {
                    is_found = true;
                } else {
                    continue;
                }
            }
            let block = cf.get(&block_id, OPERATION)?;
            blocks.push(block);
            if blocks.len() == limit {
                break;
            }
        }

        Ok(blocks)
    }

    /// Returns the key of the last change to `substate_id` on the branch ending at `end_block`, or `None` if the
    /// branch contains no change for it. If `version` is given, only changes to that version are considered.
    ///
    /// Changes recorded by blocks that are not in the branch (forked-out siblings, other subtrees) are never
    /// considered. A substate version is only ever DOWNed after it is UPed, so a DOWN supersedes the UP of the same
    /// version.
    fn block_diffs_select_last_change(
        &self,
        end_block: &BlockId,
        substate_id: &SubstateId,
        version: Option<u32>,
    ) -> Result<Option<BlockDiffKey>, RocksDbStorageError> {
        let applicable_blocks = self.get_pending_chain_until(end_block)?;

        let query = self.db().cf(block_diff::BySubstateIdQuery)?;
        let iter = query.query_prefix_range_key_iterator(Ordering::default(), substate_id);

        let mut last_change = None::<BlockDiffKey>;
        for result in iter {
            let key = result?;
            if version.is_some_and(|v| v != key.version) || !applicable_blocks.contains(&key.block_id) {
                continue;
            }
            // A DOWN is the last change a version can have, so with the version fixed there is nothing left to find.
            if version.is_some() && !key.is_up {
                return Ok(Some(key));
            }
            if last_change
                .as_ref()
                .is_none_or(|c| block_diff_change_order(c) < block_diff_change_order(&key))
            {
                last_change = Some(key);
            }
        }

        Ok(last_change)
    }

    pub fn get_commit_block(&self) -> Result<CommitBlock, RocksDbStorageError> {
        let cf = self.db().cf(CommitBlockCf)?;
        let value = cf.get_by_default_key("get_commit_block")?;
        Ok(value)
    }
}

impl<'tx, TAddr: NodeAddressable + Serialize + DeserializeOwned + 'tx, R: RocksReader> StateStoreReadTransaction
    for RocksDbStateStoreReadTransaction<'tx, TAddr, R>
{
    type Addr = TAddr;

    fn current_epoch(&self) -> Result<Epoch, StorageError> {
        let high_pc = self.db().cf(HighPcCf)?.get_by_default_key("current_epoch").optional()?;
        Ok(high_pc.map(|hpc| hpc.epoch()).unwrap_or(Epoch(0)))
    }

    fn last_sent_vote_get(&self, epoch: Epoch) -> Result<LastSentVote, StorageError> {
        let last_voted = self.db().cf(LastSentVoteCf)?.get_by_default_key("last_sent_vote_get")?;
        if last_voted.epoch() != epoch {
            return Err(StorageError::NotFound {
                item: "LastSentVote",
                key: format!("epoch {epoch}"),
            });
        }
        Ok(last_voted)
    }

    fn last_voted_get(&self, epoch: Epoch) -> Result<LastVoted, StorageError> {
        let last_voted = self.db().cf(LastVotedCf)?.get_by_default_key("last_voted_get")?;
        if last_voted.epoch() != epoch {
            return Err(StorageError::NotFound {
                item: "LastVoted",
                key: format!("epoch {epoch}"),
            });
        }

        Ok(last_voted)
    }

    fn last_executed_get(&self, epoch: Epoch) -> Result<LastExecuted, StorageError> {
        let last_executed = self.db().cf(LastExecutedCf)?.get_by_default_key("last_executed_get")?;
        if last_executed.epoch != epoch {
            return Err(StorageError::NotFound {
                item: "LastExecuted",
                key: format!("epoch {epoch}"),
            });
        }
        Ok(last_executed)
    }

    fn last_proposed_get(&self, epoch: Epoch) -> Result<LastProposed, StorageError> {
        let last_proposed = self.db().cf(LastProposedCf)?.get_by_default_key("last_proposed_get")?;
        if last_proposed.epoch != epoch {
            return Err(StorageError::NotFound {
                item: "LastProposed",
                key: format!("epoch {epoch}"),
            });
        }
        Ok(last_proposed)
    }

    fn locked_block_get(&self, epoch: Epoch) -> Result<LockedBlock, StorageError> {
        let locked_block = self.db().cf(LockedBlockCf)?.get_by_default_key("locked_block_get")?;
        if locked_block.epoch != epoch {
            return Err(StorageError::NotFound {
                item: "LockedBlock",
                key: format!("epoch {epoch}"),
            });
        }
        Ok(locked_block)
    }

    fn leaf_block_get(&self, epoch: Epoch) -> Result<LeafBlock, StorageError> {
        let leaf_block = self.leaf_block_get_any()?;
        if leaf_block.epoch() != epoch {
            return Err(StorageError::NotFound {
                item: "LeafBlock",
                key: format!("epoch {epoch}"),
            });
        }
        Ok(leaf_block)
    }

    fn leaf_block_get_any(&self) -> Result<LeafBlock, StorageError> {
        Ok(self.db().cf(LeafBlockCf)?.get_by_default_key("leaf_block_get_any")?)
    }

    fn highest_seen_block_get(&self, epoch: Epoch) -> Result<HighestSeenBlock, StorageError> {
        let last_seen_block = self
            .db()
            .cf(HighestSeenBlockCf)?
            .get_by_default_key("highest_seen_block_get")?;
        if last_seen_block.epoch() != epoch {
            return Err(StorageError::NotFound {
                item: "HighestSeenBlock",
                key: format!("epoch {epoch}"),
            });
        }
        Ok(last_seen_block)
    }

    fn last_sent_new_view_get(&self, epoch: Epoch) -> Result<LastSentNewView, StorageError> {
        let last_sent_new_view = self
            .db()
            .cf(LastSentNewViewCf)?
            .get_by_default_key("last_send_new_view")?;

        if last_sent_new_view.epoch() != epoch {
            return Err(StorageError::NotFound {
                item: "LastSentNewView",
                key: format!("epoch {epoch}"),
            });
        }

        Ok(last_sent_new_view)
    }

    fn high_pc_get(&self, epoch: Epoch) -> Result<HighPc, StorageError> {
        let high_qc = self.high_pc_get_any()?;
        if high_qc.epoch != epoch {
            return Err(StorageError::NotFound {
                item: "HighQc",
                key: format!("epoch {epoch}"),
            });
        }
        Ok(high_qc)
    }

    fn high_pc_get_any(&self) -> Result<HighPc, StorageError> {
        Ok(self.db().cf(HighPcCf)?.get_by_default_key("high_pc_get_any")?)
    }

    fn high_tc_get(&self, epoch: Epoch) -> Result<HighTc, StorageError> {
        let high_tc = self.db().cf(HighTcCf)?.get_by_default_key("high_tc_get")?;
        if high_tc.epoch != epoch {
            return Err(StorageError::NotFound {
                item: "HighTc",
                key: format!("epoch {epoch}"),
            });
        }
        Ok(high_tc)
    }

    fn is_block_in_end_of_epoch_chain(&self, block_id: &BlockId) -> Result<bool, StorageError> {
        const OPERATION: &str = "is_block_in_end_of_epoch_chain";
        let block_cf = self.db().cf(BlockCf)?;
        let chain = self.get_pending_chain_ordered(block_id)?;
        for block in chain {
            let block = block_cf.get(&block, OPERATION)?;
            if block.is_epoch_end() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn foreign_proposals_get_any<'a, I: IntoIterator<Item = &'a BlockId>>(
        &self,
        block_ids: I,
    ) -> Result<Vec<ForeignProposalRecord>, StorageError> {
        const OPERATION: &str = "foreign_proposals_get_any";
        let mut block_ids = block_ids.into_iter().peekable();
        if block_ids.peek().is_none() {
            return Ok(vec![]);
        }

        let proposals = self.db().cf(ForeignProposalCf)?.multi_get(block_ids, OPERATION)?;

        Ok(proposals)
    }

    fn foreign_proposals_exists(&self, block_id: &BlockId) -> Result<bool, StorageError> {
        let exists = self
            .db()
            .cf(ForeignProposalCf)?
            .exists(block_id, "foreign_proposals_exists")?;
        Ok(exists)
    }

    fn foreign_proposals_has_unconfirmed(&self, epoch: Epoch) -> Result<bool, StorageError> {
        let cf = self.db().cf(foreign_proposal::UnconfirmedIndexEpochQuery)?;
        let start = cf.encode_key(&Epoch::zero());
        let end = cf.encode_key(&(epoch + Epoch(1)));
        let exists = cf.any_exists_within_range(start..end)?;

        Ok(exists)
    }

    fn foreign_proposals_get_all_new(
        &self,
        block_id: &BlockId,
        limit: usize,
    ) -> Result<Vec<ForeignProposalRecord>, StorageError> {
        const OPERATION: &str = "foreign_proposals_get_all_new";

        if !self.blocks_exists(block_id)? {
            return Err(StorageError::QueryError {
                reason: format!("{OPERATION}: Block {} does not exist", block_id),
            });
        }

        let locked = self.get_current_locked_block()?;
        let pending_block_ids = self.get_pending_chain_with_commands_between(block_id)?;

        let cf = self.db().cf(ForeignProposalCf)?;
        let unconfirmed_cf = self.db().cf(foreign_proposal::UnconfirmedIndex)?;
        let proposed_in_block_cf = self.db().cf(foreign_proposal::ProposedInBlockIndex)?;
        let iter = unconfirmed_cf.key_iterator(Ordering::Ascending, OPERATION);

        let mut proposals = vec![];

        for result in iter {
            let (epoch, block_id) = result?;
            // Since the iterator is ordered by epoch, we're done since
            // we don't want to propose FPs from future epochs until we've progressed to that epoch
            if epoch > locked.epoch {
                break;
            }

            // Any pending proposed this block?
            let mut already_proposed_in_chain = false;
            for pending_block_id in &pending_block_ids {
                if proposed_in_block_cf.exists(&(*pending_block_id, block_id), OPERATION)? {
                    already_proposed_in_chain = true;
                    break;
                }
            }
            if already_proposed_in_chain {
                continue;
            }

            let proposal = cf.get(&block_id, OPERATION)?;
            proposals.push(proposal);
            if proposals.len() >= limit {
                break;
            }
        }

        Ok(proposals)
    }

    fn transactions_get(&self, tx_id: &TransactionId) -> Result<TransactionRecord, StorageError> {
        const OPERATION: &str = "transactions_get";
        let tx = self.db().cf(TransactionCf)?.get(tx_id, OPERATION)?;
        Ok(tx)
    }

    fn transactions_exists(&self, tx_id: &TransactionId) -> Result<bool, StorageError> {
        const OPERATION: &str = "transactions_exists";
        let exists = self.db().cf(TransactionCf)?.exists(tx_id, OPERATION)?;
        Ok(exists)
    }

    fn transactions_get_any<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &self,
        tx_ids: I,
    ) -> Result<Vec<TransactionRecord>, StorageError> {
        const OPERATION: &str = "transactions_get_any";
        let txs = self.db().cf(TransactionCf)?.multi_get(tx_ids, OPERATION)?;
        Ok(txs)
    }

    fn finalized_transaction_execution_get(&self, tx_id: &TransactionId) -> Result<TransactionExecution, StorageError> {
        const OPERATION: &str = "transaction_executions_get";

        let finalized_cf = self.db().cf(FinalizedTransactionLinkCf)?;
        if !finalized_cf.exists(tx_id, OPERATION)? {
            return Err(StorageError::NotFound {
                item: "TransactionExecution",
                key: format!("{tx_id}"),
            });
        }
        let cf = self.db().cf(block_transaction_execution::ByTransactionIdQuery)?;
        let mut iter = cf.query_prefix_range_key_iterator(Ordering::default(), tx_id);
        let Some((tx_id, block_id, height)) = iter.next().transpose()? else {
            return Err(StorageError::NotFound {
                item: "TransactionExecution",
                key: format!("{tx_id}"),
            });
        };
        let execution = self
            .db()
            .cf(BlockTransactionExecutionCf)?
            .get(&(tx_id, block_id, height), OPERATION)?;

        Ok(execution.into_transaction_execution())
    }

    fn finalized_transaction_execution_get_finalized_time(
        &self,
        tx_id: &TransactionId,
    ) -> Result<PrimitiveDateTime, StorageError> {
        const OPERATION: &str = "finalized_transaction_execution_get_finalized_time";
        let cf = self.db().cf(FinalizedTransactionLinkCf)?;
        let data = cf.get(tx_id, OPERATION)?;
        Ok(data.finalized_at)
    }

    fn block_transaction_executions_get_pending_for_block(
        &self,
        transaction_id: &TransactionId,
        from_block: &LeafBlock,
    ) -> Result<BlockTransactionExecution, StorageError> {
        const OPERATION: &str = "block_transaction_executions_get_pending_for_block";

        if !self.blocks_exists(from_block.block_id())? {
            return Err(StorageError::QueryError {
                reason: format!("{OPERATION}: Block {from_block} does not exist",),
            });
        }

        let cf = self.db().cf(BlockTransactionExecutionCf)?;

        // Is the execution is in the queried block
        if let Some(exec) = cf
            .get(
                &(*transaction_id, *from_block.block_id(), from_block.height()),
                OPERATION,
            )
            .optional()?
        {
            return Ok(exec);
        }

        // Collect every execution recorded for this transaction.
        let query = self.db().cf(block_transaction_execution::ByTransactionIdQuery)?;

        // An execution may only be reused if it was recorded on an ancestor of `from_block`. This is determined with
        // index lookups only - no blocks are loaded:
        //  - a block on `from_block`'s pending chain is an uncommitted ancestor, so reusable;
        //  - a block no longer in the pending-chain index has been committed, and because the committed chain is final
        //    and linear every committed block is an ancestor, so reusable;
        //  - a block still in the pending-chain index but not on `from_block`'s chain is an uncommitted
        //    orphan/abandoned branch (e.g. left over across a restart) and must NOT be reused - its execution is pinned
        //    to already-spent input versions.
        let pending_chain = self.get_pending_chain_until(from_block.block_id())?;
        let pending_index = self.db().cf(chain::PendingChainIndex)?;

        let mut best: Option<(TransactionId, BlockId, NodeHeight)> = None;
        let candidates = query.query_prefix_range_key_iterator(Ordering::default(), transaction_id);
        for res in candidates {
            let (tx_id, block_id, height) = res?;
            let is_ancestor = pending_chain.contains(&block_id) || !pending_index.exists(&block_id, OPERATION)?;
            if is_ancestor && best.as_ref().is_none_or(|(_, _, h)| *h < height) {
                best = Some((tx_id, block_id, height));
            }
        }

        if let Some((tx_id, block_id, height)) = best {
            debug!(
                target: LOG_TARGET,
                "{OPERATION}: Found execution for {transaction_id} in {block_id} {height}",
            );
            let execution = cf.get(&(tx_id, block_id, height), OPERATION)?;
            return Ok(execution);
        }

        Err(StorageError::NotFound {
            item: "TransactionExecution",
            key: format!("{transaction_id} in {from_block}"),
        })
    }

    fn block_transaction_executions_get_all_for_block(
        &self,
        block_id: &BlockId,
    ) -> Result<Vec<BlockTransactionExecution>, StorageError> {
        const OPERATION: &str = "block_transaction_executions_get_all_for_block";

        let query = self.db().cf(block_transaction_execution::ByBlockQuery)?;
        let cf = self.db().cf(BlockTransactionExecutionCf)?;

        let mut executions = Vec::new();
        for result in query.query_prefix_range_key_iterator(Ordering::default(), block_id) {
            let (block_id, tx_id, height) = result?;
            let execution = cf.get(&(tx_id, block_id, height), OPERATION)?;
            executions.push(execution);
        }

        Ok(executions)
    }

    fn blocks_get(&self, block_id: &BlockId) -> Result<Block, StorageError> {
        const OPERATION: &str = "blocks_get";

        let block = self.db().cf(BlockCf)?.get(block_id, OPERATION)?;
        Ok(block)
    }

    fn blocks_get_all_ids_by_height(&self, epoch: Epoch, height: NodeHeight) -> Result<Vec<BlockId>, StorageError> {
        // const OPERATION: &str = "blocks_get_all_ids_by_height";

        let query = self.db().cf(block::ByEpochHeightQuery)?;
        let iter = query.query_prefix_range_key_iterator(Ordering::Ascending, &(epoch, height));

        let block_ids = iter
            .map(|r| r.map(|(_, _, block_id)| block_id))
            .collect::<Result<_, _>>()?;

        Ok(block_ids)
    }

    fn blocks_get_genesis_for_epoch(&self, epoch: Epoch) -> Result<Block, StorageError> {
        // const OPERATION: &str = "blocks_get_genesis_for_epoch";

        let query = self.db().cf(block::ByEpochHeightQuery)?;
        let mut iter = query.query_prefix_range_key_iterator(Ordering::Ascending, &(epoch, NodeHeight::zero()));

        let key = iter.next().ok_or_else(|| StorageError::NotFound {
            item: "Block",
            key: format!("Genesis block for epoch {epoch}"),
        })??;

        let (_, _, block_id) = key;
        let block = self.blocks_get(&block_id)?;
        Ok(block)
    }

    fn blocks_get_all_between(
        &self,
        query_epoch: Epoch,
        start_block_height: NodeHeight,
        end_block_height: NodeHeight,
        include_dummy_blocks: bool,
        limit: usize,
    ) -> Result<Vec<Block>, StorageError> {
        const OPERATION: &str = "blocks_get_all_between";

        // Prevent the possibility of memory exhaustion (defensive, not in response to an observed bug)
        if limit > 1_000 {
            return Err(StorageError::QueryError {
                reason: format!("{OPERATION}: limit {limit} is too large"),
            });
        }

        if start_block_height > end_block_height {
            return Err(StorageError::QueryError {
                reason: format!(
                    "{OPERATION}: Start block height {start_block_height} must be less than end block height \
                     {end_block_height}"
                ),
            });
        }

        let query = self.db().cf(block::ByEpochHeightQuery)?;

        let iter = query.query_start_range_key_iterator(Ordering::Ascending, &(query_epoch, start_block_height));

        // For reasonable limits, the limit will almost always be reached, so allocate the full vector once.
        let mut blocks = Vec::with_capacity(limit);
        for result in iter {
            let (epoch, height, block_id) = result?;
            if epoch != query_epoch {
                break;
            }
            if height > end_block_height {
                break;
            }
            let block = self.blocks_get(&block_id)?;
            if !include_dummy_blocks && block.is_dummy() {
                continue;
            }
            blocks.push(block);
            if blocks.len() == limit {
                break;
            }
        }

        Ok(blocks)
    }

    fn blocks_exists(&self, block_id: &BlockId) -> Result<bool, StorageError> {
        let exists = self.db().cf(BlockCf)?.exists(block_id, "blocks_exists")?;
        Ok(exists)
    }

    fn blocks_is_pending_ancestor(&self, descendant: &BlockId, ancestor: &BlockId) -> Result<bool, StorageError> {
        const OPERATION: &str = "blocks_is_ancestor";
        // Defensive checks, technically not needed as this will return false if the blocks do not exist.
        // We could remove them to save some reads on the blocks CF.
        if !self.blocks_exists(descendant)? {
            return Err(StorageError::QueryError {
                reason: format!("{OPERATION}: descendant block {} does not exist", descendant),
            });
        }

        if !self.blocks_exists(ancestor)? {
            return Err(StorageError::QueryError {
                reason: format!("{OPERATION}: ancestor block {} does not exist", ancestor),
            });
        }

        // NOTE: This only works for non-committed/pending blocks - we only use this for the safenode predicate where
        // the ancestor block is the locked block and so is in the pending chain. Therefore, the pending chain
        // index is sufficient.
        let chain_cf = self.db().cf(chain::PendingChainIndex)?;

        let mut block_id = *descendant;
        while let Some(parent) = chain_cf.get(&block_id, OPERATION).optional()? {
            if parent == *ancestor {
                return Ok(true);
            }

            // The zero block has itself as a parent,which would cause an infinite loop, exit the loop
            if parent == block_id {
                break;
            }
            block_id = parent;
        }

        Ok(false)
    }

    fn blocks_get_committed_by_parent(&self, parent_id: &BlockId) -> Result<Block, StorageError> {
        const OPERATION: &str = "blocks_get_all_by_parent";
        // TODO: this is the only use of the chain index- change block sync to not need this
        let chain_cf = self.db().cf(chain::CommittedParentChildChainIndex)?;
        let child = chain_cf.get(parent_id, OPERATION)?;
        self.blocks_get(&child)
    }

    fn blocks_get_pending_ids_by_parent(&self, parent_id: &BlockId) -> Result<Vec<BlockId>, StorageError> {
        // const OPERATION: &str = "blocks_get_pending_ids_by_parent";

        let chain_cf = self.db().cf(chain::ByParentIdQuery)?;
        let iter = chain_cf.query_prefix_range_key_iterator(Ordering::default(), parent_id);

        let mut block_ids = vec![];
        for result in iter {
            let (_, child) = result?;
            block_ids.push(child);
        }

        Ok(block_ids)
    }

    fn blocks_get_paginated(
        &self,
        limit: u64,
        offset: u64,
        filter_index: Option<usize>,
        filter: Option<String>,
        ordering_index: Option<usize>,
        ordering: Option<Ordering>,
    ) -> Result<Vec<Block>, StorageError> {
        const OPERATION: &str = "blocks_get_paginated";
        // This operation is implemented in a naive way, by manually looping all blocks in the database.
        // This is only used for JSON-RPC get_blocks. This does not scale well.

        let block_filter = |block: &Block| {
            let Some(filter) = &filter else {
                return true;
            };
            if filter.is_empty() {
                return true;
            }
            let Some(filter_index) = filter_index else {
                return true;
            };
            match filter_index {
                0 => block.id().to_string().contains(filter),
                1 => {
                    let Ok(epoch_number) = filter.parse::<u64>() else {
                        return false;
                    };
                    block.epoch() == Epoch(epoch_number)
                },
                2 => {
                    let Ok(height_number) = filter.parse::<u64>() else {
                        return false;
                    };
                    block.height() == NodeHeight(height_number)
                },
                4 => {
                    let Ok(cmd_number) = filter.parse::<usize>() else {
                        return false;
                    };
                    block.command_count() >= cmd_number
                },
                5 => {
                    let Ok(fee) = filter.parse::<u64>() else {
                        return false;
                    };
                    block.total_leader_fee() >= fee
                },
                7 => block.proposed_by().to_string().contains(filter),
                _ => true,
            }
        };

        let Some(locked) = self.get_current_locked_block().optional()? else {
            return Ok(vec![]);
        };

        let cf = self.db().cf(BlockCf)?;
        let query = self.db().cf(block::ByEpochHeightQuery)?;

        // NOTE: this only fetches for current epoch
        let ordering = ordering.unwrap_or(Ordering::Ascending);
        let iter: Box<dyn Iterator<Item = Result<_, _>>> = if ordering.is_ascending() {
            Box::new(query.query_start_range_key_iterator(ordering, &(locked.epoch, NodeHeight(offset))))
        } else {
            let leaf_block = self.db().cf(LeafBlockCf)?.get_by_default_key(OPERATION)?;
            Box::new(query.query_end_range_key_iterator(
                ordering,
                &(
                    locked.epoch,
                    NodeHeight(leaf_block.height.as_u64().saturating_sub(offset).saturating_add(1)),
                ),
            ))
        };

        let mut blocks = vec![];
        for result in iter {
            let (epoch, _, block_id) = result?;
            if epoch != locked.epoch {
                break;
            }

            let block = cf.get(&block_id, OPERATION)?;

            if block_filter(&block) {
                blocks.push(block);
                if blocks.len() >= limit as usize {
                    break;
                }
            }
        }

        // ordering
        match ordering_index {
            Some(0) => blocks.sort_by(|a, b| a.id().cmp(b.id())),
            Some(1) => blocks.sort_by_key(|a| a.epoch()),
            // Natural sorting
            Some(2) => {}, // blocks.sort_by_key(|a| (a.epoch(), a.height())),
            Some(4) => blocks.sort_by_key(|a| a.command_count()),
            Some(5) => blocks.sort_by_key(|a| a.total_leader_fee()),
            Some(6) => blocks.sort_by_key(|a| a.block_time()),
            // TODO: This filter is by creation time, but we don't have a created_at field
            Some(7) => (),
            Some(8) => blocks.sort_by(|a, b| a.proposed_by().cmp(b.proposed_by())),
            _ => blocks.sort_by_key(|a| (a.epoch(), a.height())),
        }

        // Rocks will already order by (epoch, height)
        if ordering_index.is_some_and(|i| i != 2) && ordering.is_descending() {
            blocks.reverse();
        }

        Ok(blocks)
    }

    fn filtered_blocks_get_count(
        &self,
        filter_index: Option<usize>,
        filter: Option<String>,
    ) -> Result<u64, StorageError> {
        const OPERATION: &str = "filtered_blocks_get_count";

        let block_filter = |block: &Block| {
            let mut res = true;
            if let Some(filter) = &filter &&
                !filter.is_empty() &&
                let Some(filter_index) = filter_index
            {
                match filter_index {
                    0 => res = block.id().to_string().contains(filter),
                    1 => {
                        let epoch_number = filter.parse::<u64>().unwrap();
                        res = block.epoch() == Epoch(epoch_number);
                    },
                    2 => {
                        let height_number = filter.parse::<u64>().unwrap();
                        res = block.height() == NodeHeight(height_number);
                    },
                    4 => {
                        let cmd_number = filter.parse::<usize>().unwrap();
                        res = block.command_count() >= cmd_number;
                    },
                    5 => {
                        let fee = filter.parse::<u64>().unwrap();
                        res = block.total_leader_fee() >= fee;
                    },
                    7 => res = block.proposed_by().to_string().contains(filter),
                    _ => (),
                }
            }
            res
        };

        let cf = self.db().cf(BlockCf)?;

        let no_filters = filter_index.is_none() || filter.as_ref().is_none_or(|x| x.is_empty());
        if no_filters {
            let count = cf.count(OPERATION)?;
            return Ok(count as u64);
        }

        let iter = cf.value_iterator(Ordering::Ascending, OPERATION);

        let mut count = 0;
        for result in iter {
            let block = result?;
            if block_filter(&block) {
                count += 1;
            }
        }

        Ok(count)
    }

    fn block_diffs_get(&self, block_id: &BlockId) -> Result<BlockDiff, StorageError> {
        // const OPERATION: &str = "block_diffs_get";
        let cf = self.db().cf(block_diff::ByBlockIdQuery)?;
        let iter = cf.query_prefix_range_value_iterator(Ordering::default(), block_id);
        let mut changes = Vec::new();
        for result in iter {
            let change = result?;
            changes.push(change);
        }

        let diff = BlockDiff::new(*block_id, changes);
        Ok(diff)
    }

    fn block_diffs_get_last_change_for_substate(
        &self,
        block_id: &BlockId,
        substate_id: &SubstateId,
    ) -> Result<SubstateChange, StorageError> {
        const OPERATION: &str = "block_diffs_get_last_change_for_substate";
        if !self.blocks_exists(block_id)? {
            return Err(StorageError::QueryError {
                reason: format!("{OPERATION}: Block {} does not exist", block_id),
            });
        }

        let key = self
            .block_diffs_select_last_change(block_id, substate_id, None)?
            .ok_or_else(|| StorageError::NotFound {
                item: "SubstateChange",
                key: format!("{substate_id} in {block_id}"),
            })?;

        let change = self.db().cf(BlockDiffCf)?.get(&key, OPERATION)?;
        Ok(change)
    }

    fn block_diffs_get_change_for_versioned_substate<'a, T: Into<VersionedSubstateIdRef<'a>>>(
        &self,
        block_id: &BlockId,
        substate_id: T,
    ) -> Result<SubstateChange, StorageError> {
        const OPERATION: &str = "block_diffs_get_change_for_versioned_substate";
        if !self.blocks_exists(block_id)? {
            return Err(StorageError::QueryError {
                reason: format!("{OPERATION}: Block {} does not exist", block_id),
            });
        }

        let versioned = substate_id.into();

        let key = self
            .block_diffs_select_last_change(block_id, versioned.substate_id(), Some(versioned.version()))?
            .ok_or_else(|| StorageError::NotFound {
                item: "SubstateChange",
                key: format!("{versioned} in {block_id}"),
            })?;

        let change = self.db().cf(BlockDiffCf)?.get(&key, OPERATION)?;
        Ok(change)
    }

    fn proposal_certificates_get(&self, epoch: Epoch, qc_id: &PcId) -> Result<ProposalCertificate, StorageError> {
        const OPERATION: &str = "proposal_certificates_get";
        let qc = self.db().cf(ProposalCertificateCf)?.get(&(epoch, *qc_id), OPERATION)?;
        Ok(qc)
    }

    fn proposal_certificates_get_many<'a, I>(&self, qc_ids: I) -> Result<Vec<ProposalCertificate>, StorageError>
    where
        I: IntoIterator<Item = &'a (Epoch, PcId)>,
        I::IntoIter: ExactSizeIterator,
    {
        const OPERATION: &str = "proposal_certificates_get_all";
        let iter = qc_ids.into_iter();
        let expected = iter.len();
        let qcs = self.db().cf(ProposalCertificateCf)?.multi_get(iter, OPERATION)?;
        if qcs.len() != expected {
            return Err(StorageError::NotFound {
                item: "QuorumCertificate",
                key: "one or more qc_ids".to_string(),
            });
        }
        Ok(qcs)
    }

    fn timeout_certificates_get(&self, epoch: Epoch, id: &TcId) -> Result<TimeoutCertificate, StorageError> {
        const OPERATION: &str = "timeout_certificates_get";
        let tc = self.db().cf(TimeoutCertificateCf)?.get(&(epoch, *id), OPERATION)?;
        Ok(tc)
    }

    fn timeout_certificates_get_many<'a, I>(&self, ids: I) -> Result<Vec<TimeoutCertificate>, StorageError>
    where
        I: IntoIterator<Item = &'a (Epoch, TcId)>,
        I::IntoIter: ExactSizeIterator,
    {
        const OPERATION: &str = "timeout_certificates_get_many";
        let iter = ids.into_iter();
        let expected = iter.len();
        let tcs = self.db().cf(TimeoutCertificateCf)?.multi_get(iter, OPERATION)?;
        if tcs.len() != expected {
            return Err(StorageError::NotFound {
                item: "TimeoutCertificate",
                key: "one or more tc_ids".to_string(),
            });
        }
        Ok(tcs)
    }

    fn transaction_pool_get_for_blocks(
        &self,
        to_block_id: &BlockId,
        transaction_id: &TransactionId,
    ) -> Result<TransactionPoolRecord, StorageError> {
        const OPERATION: &str = "transaction_pool_get_for_blocks";
        if !self.blocks_exists(to_block_id)? {
            return Err(StorageError::QueryError {
                reason: format!("transaction_pool_get_for_blocks: Block {} does not exist", to_block_id),
            });
        }

        let cf = self.db().cf(TransactionPoolCf)?;
        let query = self
            .db()
            .cf(transaction_pool_state_update::TransactionPoolStateUpdateCf)?;

        let mut transaction = cf.get(transaction_id, OPERATION)?;

        let pending_chain = self.get_pending_chain_ordered(to_block_id)?;

        trace!(
            target: LOG_TARGET,
            "{OPERATION}: pending_chain: {} for block {}",
            pending_chain.display(), to_block_id
        );

        for block_id in pending_chain {
            let mut iter = query.prefix_range_value_iterator(Ordering::default(), &(block_id, *transaction_id));
            if let Some(update) = iter.next().transpose()? {
                debug!(
                    target: LOG_TARGET,
                    "{OPERATION}: found update {} for block {}: {:#} -> {:#}",
                    update.transaction_id, block_id,
                    transaction.evidence(),
                    update.evidence
                );
                update.merge_into(&mut transaction);
                return Ok(transaction);
            }
        }

        Ok(transaction)
    }

    fn transaction_pool_exists(&self, transaction_id: &TransactionId) -> Result<bool, StorageError> {
        let exists = self
            .db()
            .cf(TransactionPoolCf)?
            .exists(transaction_id, "transaction_pool_exists")?;
        Ok(exists)
    }

    fn transaction_pool_get_all(&self, limit: usize) -> Result<Vec<TransactionPoolRecord>, StorageError> {
        const OPERATION: &str = "transaction_pool_get_all";
        let cf = self.db().cf(TransactionPoolCf)?;

        // Overlay each record's latest pending update by walking the pending chain back from the leaf block, so
        // callers see its effective state at the tip. With no leaf block yet (e.g. freshly state-synced, before
        // joining consensus) there is no pending chain and nothing to overlay.
        let mut updates = HashMap::new();
        if let Some(leaf_block) = self.leaf_block_get_any().optional()? {
            let pending_chain = self.get_pending_chain_ordered(leaf_block.block_id())?;
            let query = self.db().cf(transaction_pool_state_update::ByBlockIdQuery)?;
            // Oldest first so the tip's update wins per transaction.
            for block_id in pending_chain.into_iter().rev() {
                let iter = query.query_prefix_range_iterator(Ordering::default(), &block_id);
                for result in iter {
                    let ((_, tx_id), update) = result?;
                    updates.insert(tx_id, update);
                }
            }
        }

        let n = cf.count(OPERATION)?;
        let iter = cf.value_iterator(Ordering::Ascending, OPERATION);
        let mut transactions = Vec::with_capacity(n.min(limit));
        for result in iter {
            let mut tx = result?;
            if let Some(update) = updates.remove(tx.id()) {
                update.merge_into(&mut tx);
            }
            transactions.push(tx);
            if transactions.len() == limit {
                break;
            }
        }
        Ok(transactions)
    }

    fn transaction_pool_get_many_ready(
        &self,
        weight_budget: u64,
        max_count: usize,
        block_id: &BlockId,
    ) -> Result<Vec<TransactionPoolRecord>, StorageError> {
        const OPERATION: &str = "transaction_pool_get_many_ready";
        if !self.blocks_exists(block_id)? {
            return Err(StorageError::QueryError {
                reason: format!("transaction_pool_get_for_blocks: Block {} does not exist", block_id),
            });
        }

        let cf = self.db().cf(TransactionPoolCf)?;

        let query = self.db().cf(transaction_pool_state_update::ByBlockIdQuery)?;
        let lock_conflicts_cf = self.db().cf(lock_conflict::ByTransactionIdQuery)?;

        let pending_chain = self.get_pending_chain_ordered(block_id)?;

        // TODO: optimise
        let mut updates = HashMap::new();
        let mut lock_conflicted = HashSet::new();
        for block_id in pending_chain.into_iter().rev() {
            let iter = query.query_prefix_range_iterator(Ordering::default(), &block_id);
            for result in iter {
                let ((_, tx_id), update) = result?;
                if lock_conflicts_cf.exists_prefix(&tx_id)? {
                    lock_conflicted.insert(tx_id);
                    continue;
                }
                updates.insert(tx_id, update);
            }
        }

        let mut transactions = Vec::new();
        let mut accumulated_weight = 0u64;
        let iter = cf.value_iterator(Ordering::default(), OPERATION);

        for result in iter {
            let mut tx = result?;
            if lock_conflicted.contains(tx.id()) {
                continue;
            }

            if lock_conflicts_cf.exists_prefix(tx.id())? {
                continue;
            }
            if let Some(update) = updates.remove(tx.id()) {
                update.merge_into(&mut tx);
            }
            if tx.is_ready() {
                // Always include at least one ready record so a transaction heavier than the whole
                // budget still makes progress, then stop once the budget or count cap is reached.
                let next_weight = accumulated_weight.saturating_add(tx.proposal_weight());
                if !transactions.is_empty() && next_weight > weight_budget {
                    break;
                }
                accumulated_weight = next_weight;
                transactions.push(tx);

                if transactions.len() >= max_count {
                    break;
                }
            }
        }

        Ok(transactions)
    }

    fn transaction_pool_has_pending_state_updates(&self, block_id: &BlockId) -> Result<bool, StorageError> {
        // const OPERATION: &str = "transaction_pool_has_pending_state_updates";
        let pending_chain = self.get_pending_chain_ordered(block_id)?;
        let query = self.db().cf(transaction_pool_state_update::ByBlockIdQuery)?;
        for block_id in pending_chain {
            if query.exists_prefix(&block_id)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn transaction_pool_count(
        &self,
        stage: Option<TransactionPoolStage>,
        is_ready: Option<bool>,
        skip_lock_conflicted: bool,
    ) -> Result<usize, StorageError> {
        const OPERATION: &str = "transaction_pool_count";

        let cf = self.db().cf(TransactionPoolCf)?;

        let lock_conflict_query = self.db().cf(lock_conflict::ByTransactionIdQuery)?;
        let iter = cf.key_iterator(Ordering::default(), OPERATION);
        let mut count = 0;

        let must_query = stage.is_some() || is_ready.is_some() || skip_lock_conflicted;

        for result in iter {
            let tx_id = result?;
            if must_query {
                let Some(tx_pool_rec) = cf.get(&tx_id, OPERATION).optional()? else {
                    // It's possible that the transaction has been removed from the pool in another thread 😱 - observed
                    // in consensus_tests
                    continue;
                };

                if let Some(stage) = stage &&
                    tx_pool_rec.current_stage() != stage
                {
                    continue;
                }

                if let Some(is_ready) = is_ready &&
                    tx_pool_rec.is_ready() != is_ready
                {
                    continue;
                }

                if skip_lock_conflicted && lock_conflict_query.exists_prefix(&tx_id)? {
                    continue;
                    // let iter = lock_conflict_query.query_prefix_range_iterator(Ordering::default(), &tx_id);
                    // for result in iter {
                    //     let (_, value) = result?;
                    //     if !value.is_local_only {
                    //         continue;
                    //     }
                    // }
                }
            }

            count += 1;
        }

        Ok(count)
    }

    fn substates_get(&self, address: &SubstateAddress) -> Result<SubstateRecord, StorageError> {
        const OPERATION: &str = "substates_get";
        let substate = self.db().cf(SubstateCf)?.get(address, OPERATION)?;
        Ok(substate)
    }

    fn substates_get_any<'a, I: IntoIterator<Item = &'a VersionedSubstateIdRef<'a>>>(
        &self,
        substate_ids: I,
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        const OPERATION: &str = "substates_get_any";

        let substates = self
            .db()
            .cf(SubstateCf)?
            .multi_get(substate_ids.into_iter().map(|id| id.to_substate_address()), OPERATION)?;

        Ok(substates)
    }

    fn substates_get_any_max_version<'a, I: IntoIterator<Item = &'a SubstateId>>(
        &self,
        substate_ids: I,
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        const OPERATION: &str = "substates_get_any_max_version";

        let index_cf = self.db().cf(substate::HeadIndex)?;
        let cf = self.db().cf(SubstateCf)?;

        let iter = substate_ids.into_iter();
        let (lower, _) = iter.size_hint();
        let mut substates = Vec::with_capacity(lower);
        for substate_id in iter {
            let Some(head) = index_cf.get(substate_id, OPERATION).optional()? else {
                debug!(
                    target: LOG_TARGET,
                    "{OPERATION}: substate_id {} not found in head index",
                    substate_id
                );
                continue;
            };
            let address = SubstateAddress::from_substate_id(substate_id, head.version);
            let substate = cf.get(&address, OPERATION)?;
            substates.push(substate);
        }

        Ok(substates)
    }

    fn substates_get_max_version_for_substate(&self, substate_id: &SubstateId) -> Result<(u32, bool), StorageError> {
        const OPERATION: &str = "substates_get_max_version_for_substate";
        let index_cf = self.db().cf(substate::HeadIndex)?;
        let data = index_cf.get(substate_id, OPERATION)?;
        Ok((data.version, data.is_up))
    }

    fn substates_exists_any_version(&self, substate_id: &SubstateId) -> Result<bool, StorageError> {
        const OPERATION: &str = "substates_exists_any_version";
        let exists = self.db().cf(substate::HeadIndex)?.exists(substate_id, OPERATION)?;
        Ok(exists)
    }

    fn substates_any_exist<'a, I>(&self, substates: I) -> Result<bool, StorageError>
    where I: IntoIterator<Item = VersionedSubstateIdRef<'a>> {
        const OPERATION: &str = "substates_any_exist";

        let cf = self.db().cf(SubstateCf)?;

        for id in substates {
            if cf.exists(&id.to_substate_address(), OPERATION)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn substate_tree_iter(
        &self,
        shard: Shard,
        starting_from_state_version: Version,
        value_filters: SubstateValueFilterFlags,
    ) -> Result<impl Iterator<Item = Result<(Version, SubstateId, Substate), StorageError>>, StorageError> {
        let query = self.db().cf(state_tree::ByShardStateVersionQuery)?;
        let substate_cf = self.db().cf(SubstateCf)?;
        Ok(LatestSubstateStateTreeIterator::new(
            query,
            substate_cf,
            shard,
            starting_from_state_version,
            value_filters,
        ))
    }

    /// Returns all substates that have been locked by a transaction.
    ///
    ///  # Used for:
    /// - fetching the local pledges for a transaction, so that they can be sent as a foreign proposal to the network
    fn substate_locks_get_locked_substates_for_transaction(
        &self,
        transaction_id: &TransactionId,
    ) -> Result<Vec<LockedSubstateValue>, StorageError> {
        const OPERATION: &str = "substate_locks_get_locked_substates_for_transaction";

        let substates_cf = self.db().cf(SubstateCf)?;
        let query = self.db().cf(substate_locks::ByTransactionIdQuery)?;

        let mut locked_substates = Vec::new();

        let iter = query.query_prefix_range_iterator(Ordering::default(), transaction_id);

        for result in iter {
            let (key, lock) = result?;
            let substate = substates_cf
                .get(
                    &SubstateAddress::from_substate_id(&key.substate_id, lock.version()),
                    OPERATION,
                )
                .optional()?;
            locked_substates.push(LockedSubstateValue {
                substate_id: key.substate_id,
                lock,
                value: substate.and_then(|s| s.into_substate_value()),
            });
        }

        Ok(locked_substates)
    }

    /// Returns the transaction ID of any write lock found for any of the given substates, or None if there are no write
    /// locks.
    ///
    /// # Used for:
    /// Foreign proposal conflict resolution, to check if there is a conflicting write lock in a foreign proposal by
    /// another locally proposed transaction.
    fn substate_locks_has_any_write_locks_for_substates<'a, I: IntoIterator<Item = &'a SubstateId>>(
        &self,
        exclude_transaction_id: Option<&TransactionId>,
        substate_ids: I,
    ) -> Result<Option<TransactionId>, StorageError> {
        // const OPERATION: &str = "substate_locks_has_any_write_locks_for_substates";
        let mut substate_ids = substate_ids.into_iter().peekable();
        if substate_ids.peek().is_none() {
            return Ok(None);
        }

        let query = self.db().cf(substate_locks::BySubstateIdQuery)?;

        for substate_id in substate_ids {
            let iter = query.query_prefix_range_iterator(Ordering::default(), substate_id);
            for result in iter {
                let (key, lock_type) = result?;
                if !lock_type.is_write() {
                    continue;
                }
                if exclude_transaction_id.is_some_and(|ex| *ex == key.transaction_id) {
                    continue;
                }

                return Ok(Some(key.transaction_id));
            }
        }

        Ok(None)
    }

    /// Returns the latest substate lock for a given substate ID, searching through the pending chain and committed
    /// chain.
    ///
    /// # Used for:
    /// Local proposal conflict resolution, to check if a substate is locked by another transaction.
    fn substate_locks_get_latest_for_substate(
        &self,
        leaf_block: &LeafBlock,
        substate_id: &SubstateId,
    ) -> Result<SubstateLock, StorageError> {
        const OPERATION: &str = "substate_locks_get_latest_for_substate";
        let cf = self.db().cf(SubstateLockModel)?;

        let pending_chain = self.get_pending_chain_ordered(leaf_block.block_id())?;

        // Check if the substate lock is in the head index (typical case optimisation)
        let head_idx = self.db().cf(substate_locks::HeadIndex)?;
        if let Some(head) = head_idx.get(substate_id, OPERATION).optional()? &&
            pending_chain.contains(&head.block_id)
        {
            let lock = cf.get(&head, OPERATION)?;
            return Ok(lock);
        }

        let query = self.db().cf(substate_locks::ByBlockIdSubstateIdQuery)?;

        // TODO: this is on the critical path, improve performance
        for block_id in &pending_chain {
            let mut iter =
                query.query_prefix_range_key_iterator(Ordering::default(), &(*block_id, substate_id.clone()));
            if let Some(result) = iter.next() {
                let key = result?;
                let lock = cf.get(&key, OPERATION)?;
                return Ok(lock);
            }
        }

        // In the committed chain? This index is ordered by (substate_id, transaction_id, block_id, height), not by
        // height, so scan every committed lock for the substate and keep the one from the highest block (the latest).
        let commit_block = self.get_commit_block()?;
        let query = self.db().cf(substate_locks::BySubstateIdQuery)?;
        let iter = query.query_prefix_range_key_iterator(Ordering::default(), substate_id);
        let mut latest: Option<substate_locks::SubstateLockKey> = None;
        for result in iter {
            let key = result?;
            if key.block_height > commit_block.height {
                continue;
            }
            let is_later = match &latest {
                Some(prev) => prev.block_height < key.block_height,
                None => true,
            };
            if is_later {
                latest = Some(key);
            }
        }
        let key = latest.ok_or_else(|| StorageError::NotFound {
            item: "SubstateLock",
            key: format!("for substate {substate_id} in block {leaf_block}"),
        })?;

        let lock = cf.get(&key, OPERATION)?;
        Ok(lock)
    }

    fn pending_state_tree_diffs_get_all_up_to_commit_block(
        &self,
        block_id: &BlockId,
    ) -> Result<HashMap<Shard, Vec<PendingShardStateTreeDiff>>, StorageError> {
        const OPERATION: &str = "pending_state_tree_diffs_get_all_up_to_commit_block";
        if !self.blocks_exists(block_id)? {
            return Err(StorageError::NotFound {
                item: "pending_state_tree_diffs_get_all_up_to_commit_block: Block",
                key: block_id.to_string(),
            });
        }

        // Block may modify state with zero commands because the justify block changes state
        let block_ids = self.get_pending_chain_ordered(block_id)?;
        if block_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let query = self.db().cf(pending_state_tree_diff::ByBlockIdQuery)?;

        let mut diffs = HashMap::new();
        // Load diffs in from earliest to latest
        for block_id in block_ids.iter().rev() {
            trace!(
                target: LOG_TARGET,
                "{OPERATION}: diffs for block {}",
                block_id
            );
            let iter = query.query_prefix_range_iterator(Ordering::default(), block_id);
            for result in iter {
                let ((_, shard), diff) = result?;
                trace!(
                    target: LOG_TARGET,
                    "{OPERATION}: got diff for shard {} (v{}, new={}, stale={})",
                    shard, diff.version, diff.diff.new_nodes.len(), diff.diff.stale_tree_nodes.len()
                );
                let diff_mut = diffs.entry(shard).or_insert_with(Vec::new);
                diff_mut.push(diff);
            }
        }

        Ok(diffs)
    }

    fn state_transitions_get_starting_at(
        &self,
        req_shard: Shard,
        state_version: Version,
        value_filter: SubstateValueFilterFlags,
    ) -> Result<StateVersionTransitions, StorageError> {
        if value_filter.is_empty() {
            return Err(StorageError::QueryError {
                reason: "state_transitions_get_starting_at: value_filter cannot be empty".to_string(),
            });
        }

        const OPERATION: &str = "state_transitions_get_n_after";

        let query = self.db().cf(state_transition::ByShardAndStateVersionQuery)?;
        let mut iter = query.query_range_iterator(
            Ordering::Ascending,
            (req_shard, state_version)..(Shard::max(), Version::MAX),
        );
        let substate_cf = self.db().cf(SubstateCf)?;

        // NOTE: The state version may not have any transitions
        let result = iter.next().ok_or_else(|| StorageError::NotFound {
            item: "StateTransition",
            key: format!("shard {} and state version {}", req_shard, state_version),
        })?;
        let ((shard, version), data) = result?;

        // If we've scanned onto the next shard, we couldn't find the requested shard
        if shard != req_shard {
            return Err(StorageError::NotFound {
                item: "StateTransition",
                key: format!("shard {} and state version {}", req_shard, state_version),
            });
        }

        // TODO(perf): we still have to load all substates for the id and version regardless of the value filters - not
        // ideal
        let substates = substate_cf.multi_get(data.transitions.iter().map(|t| t.substate_address), OPERATION)?;

        let mut updates = Vec::with_capacity(data.transitions.len());
        let all_hashes = value_filter.include_filtered_hashes();
        let up_only = value_filter.is_up_only();
        // multi_get returns the substates in the same order as queried, so ordered by transitions
        for (rec, substate) in data.transitions.iter().zip(substates) {
            let update = match rec.transition {
                StateTransitionType::Up => {
                    if up_only && substate.is_destroyed() {
                        // The caller wants the set that is live now, so an up whose substate has since been
                        // destroyed is skipped. Its down is skipped too (see below), so including it would
                        // leave the caller holding a substate nothing later in the stream takes back down.
                        None
                    } else {
                        let metadata_mode = value_filter.contains(SubstateValueFilterFlags::TEMPLATE_METADATA);
                        let template_metadata_opt = if metadata_mode && substate.substate_id.is_template() {
                            substate
                                .substate_value
                                .as_ref()
                                .and_then(|v| v.as_template())
                                .map(|t| t.clone().into_template_metadata())
                        } else {
                            None
                        };

                        let is_metadata_only = metadata_mode &&
                            !value_filter.contains(SubstateValueFilterFlags::TEMPLATE) &&
                            substate.substate_id.is_template();
                        let resolved_value = value_filter
                                .contains_substate(&substate.substate_id)
                                // filter includes substate, return full value if available (unless metadata-only for templates), otherwise return hash
                                .then(|| {
                                    substate
                                        .substate_value
                                        .filter(|_| !is_metadata_only)
                                        .map(|v| SubstateValueOrHash::Value(Box::new(v)))
                                        .unwrap_or_else(|| SubstateValueOrHash::Hash(substate.state_hash))
                                })
                                // Otherwise if the client still wants all hashes for filtered out substates
                                .or_else(|| all_hashes.then_some(SubstateValueOrHash::Hash(substate.state_hash)));

                        resolved_value.map(|value| {
                            SubstateUpdateProof::Create(Box::new(SubstateCreate {
                                substate: SubstateData {
                                    substate_id: substate.substate_id,
                                    version: substate.version,
                                    value,
                                    template_metadata: template_metadata_opt,
                                },
                            }))
                        })
                    }
                },
                StateTransitionType::Down => (!up_only)
                    .then(|| {
                        // A destroy carries no value, so `all_hashes` admits it at the cost of an id
                        // and a version: the only signal a subscriber has that a substate outside its
                        // value filter went down with no successor version.
                        (all_hashes || value_filter.contains_substate(&substate.substate_id)).then(|| {
                            SubstateUpdateProof::Destroy(SubstateDestroy {
                                substate_id: substate.substate_id,
                                version: substate.version,
                            })
                        })
                    })
                    .flatten(),
            };

            if let Some(update) = update {
                updates.push(update);
            }
        }

        Ok(StateVersionTransitions {
            epoch: data.epoch,
            shard,
            state_version: version,
            updates,
        })
    }

    fn state_tree_nodes_get(&self, shard: Shard, key: &NodeKey) -> Result<Node<StateTreePayload>, StorageError> {
        const OPERATION: &str = "state_tree_nodes_get";
        let cf = self.db().cf(StateTreeCf)?;
        // We do this to avoid a clone
        let codec = StateTreeCf::key_codec();
        let key = codec.encode(&(shard, key))?;
        let node = cf.get_raw_key(&key, OPERATION)?;
        Ok(node)
    }

    fn state_tree_nodes_get_all_by_state_version(
        &self,
        shard: Shard,
        state_version: Version,
    ) -> Result<Vec<(NodeKey, Node<StateTreePayload>)>, StorageError> {
        let cf = self.db().cf(state_tree::ByShardStateVersionQuery)?;
        let iter = cf.query_prefix_range_iterator(Ordering::default(), &(shard, state_version));
        let nodes = iter
            .map(|result| result.map(|((_, key), value)| (key, value)))
            .collect::<Result<_, _>>()?;
        Ok(nodes)
    }

    fn state_tree_versions_get_latest(&self, shard: Shard) -> Result<Option<Version>, StorageError> {
        const OPERATION: &str = "state_tree_versions_get_latest";
        let cf = self.db().cf(StateTreeShardVersionCf)?;
        let version = cf.get(&shard, OPERATION).optional()?;
        Ok(version)
    }

    fn state_tree_versions_get_latest_for_shard_group(
        &self,
        shard_group: ShardGroup,
    ) -> Result<ShardStateVersions, StorageError> {
        const OPERATION: &str = "state_tree_versions_get_latest_for_shard_group";
        let mut shard_tree_versions = vec![StateVersion::zero(); shard_group.len() + 1];

        let cf = self.db().cf(state_tree_shard_versions::ByShard)?;
        let global = cf.get(&Shard::global(), OPERATION).optional()?.unwrap_or_default();
        shard_tree_versions[0] = global.into();
        let sg_range = shard_group.start()..Shard::from(shard_group.end().as_u32() + 1);
        let iter = cf.query_range_iterator(Ordering::Ascending, sg_range);
        let mut shards_iter = shard_group.shard_iter();
        for result in iter {
            let (shard, version) = result?;

            // Fill in the gaps with 0s for shard versions that are not yet set
            for sg_shard in shards_iter.by_ref() {
                if shard == sg_shard {
                    break;
                }

                let index =
                    ShardStateVersions::shard_to_index(shard_group, sg_shard).expect("sg_shard must be in shard group");
                shard_tree_versions[index] = StateVersion::zero();
            }

            let index = ShardStateVersions::shard_to_index(shard_group, shard)
                .expect("BUG: we checked the end of the shard group, so shard must be in shard group");

            shard_tree_versions[index] = version.into();
        }

        let shard_tree_versions =
            ShardStateVersions::from_vec(shard_tree_versions).expect("BUG: more shard tree versions than shards");
        Ok(shard_tree_versions)
    }

    fn epoch_checkpoint_get_all_from_epoch(
        &self,
        from_epoch: Epoch,
        limit: usize,
    ) -> Result<Vec<EpochCheckpoint>, StorageError> {
        // const OPERATION: &str = "epoch_checkpoint_get_all";
        let query = self.db().cf(epoch_checkpoint::ByEpochQuery)?;
        let iter = query.query_range_iterator(Ordering::Ascending, from_epoch..Epoch::max());
        let epoch_checkpoints = iter
            .map(|result| result.map(|(_, checkpoint)| checkpoint))
            .take(limit)
            .collect::<Result<_, _>>()?;
        Ok(epoch_checkpoints)
    }

    fn epoch_checkpoint_get_by_shard_group(
        &self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<EpochCheckpoint, StorageError> {
        const OPERATION: &str = "epoch_checkpoint_get_by_shard_group";
        let cf = self.db().cf(EpochCheckpointCf)?;
        let checkpoint = cf.get(&(epoch, shard_group), OPERATION)?;
        Ok(checkpoint)
    }

    fn epoch_checkpoint_get_last(&self) -> Result<EpochCheckpoint, StorageError> {
        const OPERATION: &str = "epoch_checkpoint_get_last";
        let cf = self.db().cf(EpochCheckpointCf)?;
        let mut iter = cf.iterator(Ordering::Descending, OPERATION);
        let (_, checkpoint) = iter.next().ok_or_else(|| StorageError::NotFound {
            item: "EpochCheckpoint",
            key: "last".to_string(),
        })??;
        Ok(checkpoint)
    }

    fn foreign_substate_pledges_exists_for_transaction_and_address<T: ToSubstateAddress>(
        &self,
        transaction_id: &TransactionId,
        address: T,
    ) -> Result<bool, StorageError> {
        const OPERATION: &str = "foreign_substate_pledges_exists_for_transaction_and_address";
        let cf = self.db().cf(ForeignSubstatePledgeCf)?;
        let exists = cf.exists(&(*transaction_id, address.to_substate_address()), OPERATION)?;
        Ok(exists)
    }

    fn foreign_substate_pledges_get_write_pledges_to_transaction<'a, I: IntoIterator<Item = &'a SubstateId>>(
        &self,
        transaction_id: &TransactionId,
        substate_ids: I,
    ) -> Result<SubstatePledges, StorageError> {
        // const OPERATION: &str = "foreign_substate_pledges_get_write_pledges_to_transaction";

        let query = self.db().cf(foreign_substate_pledge::ByTransactionIdQuery)?;

        let mut pledges = SubstatePledges::new();
        let iter = query.query_prefix_range_iterator(Ordering::default(), transaction_id);
        let substate_ids = substate_ids.into_iter().collect::<HashSet<_>>();

        for result in iter {
            let (_, pledge) = result?;
            if !pledge.is_write() {
                continue;
            }
            if substate_ids.contains(pledge.substate_id()) {
                pledges.push(pledge);
            }
        }

        Ok(pledges)
    }

    fn foreign_substate_pledges_get_all_by_transaction_id(
        &self,
        transaction_id: &TransactionId,
    ) -> Result<SubstatePledges, StorageError> {
        // const OPERATION: &str = "foreign_substate_pledges_get_all_by_transaction_id";

        let query = self.db().cf(foreign_substate_pledge::ByTransactionIdQuery)?;

        let mut pledges = SubstatePledges::new();
        let iter = query.query_prefix_range_iterator(Ordering::default(), transaction_id);
        for result in iter {
            let (_, pledge) = result?;
            pledges.push(pledge);
        }

        Ok(pledges)
    }

    fn parked_block_exists(&self, block_id: &BlockId) -> Result<bool, StorageError> {
        const OPERATION: &str = "parked_block_exists";
        let exists = self.db().cf(parked_block::ParkedBlockCf)?.exists(block_id, OPERATION)?;
        Ok(exists)
    }

    fn foreign_parked_blocks_exists(&self, block_id: &BlockId) -> Result<bool, StorageError> {
        const OPERATION: &str = "foreign_parked_blocks_exists";
        let cf = self.db().cf(ForeignParkedBlockCf)?;
        let exists = cf.exists(block_id, OPERATION)?;
        Ok(exists)
    }

    fn validator_epoch_stats_get(
        &self,
        epoch: Epoch,
        public_key: &RistrettoPublicKeyBytes,
    ) -> Result<ValidatorConsensusStats, StorageError> {
        const OPERATION: &str = "validator_epoch_stats_get";
        let cf = self.db().cf(ValidatorNodeEpochStatsCf)?;
        let stats = cf.get(&(epoch, *public_key), OPERATION)?;
        Ok(stats)
    }
}

/// Orders two changes for the same substate within a branch. A substate version is only ever DOWNed after it is UPed,
/// so a DOWN supersedes the UP of the same version.
fn block_diff_change_order(key: &BlockDiffKey) -> (u32, bool) {
    (key.version, !key.is_up)
}
