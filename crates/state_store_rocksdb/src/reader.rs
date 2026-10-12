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
    SubstateVersion,
    ToSubstateAddress,
    VersionedSubstateId,
    VersionedSubstateIdRef,
    displayable::Displayable,
    optional::Optional,
    shard::Shard,
};
use tari_ootle_storage::{
    Ordering,
    PendingChain,
    StateStoreReadTransaction,
    StorageError,
    consensus_models::{
        Block,
        BlockDiff,
        BlockTransactionExecution,
        EpochCheckpoint,
        ForeignProposalRecord,
        LivenessCounters,
        LockedSubstateValue,
        PendingShardStateTreeDiff,
        StateVersionProof,
        StateVersionTransitions,
        SubstateChange,
        SubstateCreate,
        SubstateData,
        SubstateDestroy,
        SubstateDownProofRecord,
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
    block_diff_table::change_order,
    cf_api::DbContext,
    codecs::{DbEncoder, SubstateIdCodec},
    column_families::{
        block,
        block::BlockCf,
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
        pending_state_tree_diff::ShardStateTreeDiff,
        state_sync_rewind_point::StateSyncRewindPointCf,
        state_transition,
        state_transition::StateTransitionType,
        state_tree,
        state_tree::StateTreeCf,
        state_tree_shard_versions,
        state_tree_shard_versions::StateTreeShardVersionCf,
        state_version_proof::{BlockCommitProofCf, StateVersionProofCf},
        substate,
        substate::SubstateCf,
        substate_down_proof::SubstateDownProofCf,
        transaction::TransactionCf,
        transaction_pool::TransactionPoolCf,
        transaction_pool_state_update,
        validator_liveness_log::ValidatorLivenessLogCf,
        validator_node_epoch_stats::ValidatorNodeEpochStatsCf,
        vote_equivocation,
    },
    error::RocksDbStorageError,
    pending_state::{PendingState, PendingStateView},
    read_only::ReadOnly,
    state_tree_iterator::LatestSubstateStateTreeIterator,
    traits::{Cf, RocksReader},
};

const LOG_TARGET: &str = "tari::ootle::storage::state_store_rocksdb::reader";

pub(crate) type ReadOnlyTransaction<'a> = ReadOnly<Transaction<'a, TransactionDB>>;

pub struct RocksDbStateStoreReadTransaction<'a, TAddr, R = ReadOnlyTransaction<'a>> {
    tx: R,
    db: &'a TransactionDB,
    pending: PendingStateView<'a>,
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
    pub(crate) fn new(db: &'a TransactionDB, tx: R, pending: PendingStateView<'a>) -> Self {
        Self {
            tx,
            db,
            pending,
            _addr: PhantomData,
            _not_send: PhantomData,
        }
    }

    pub fn db(&self) -> DbContext<'_, R> {
        DbContext::new(self.db, &self.tx)
    }

    pub(crate) fn pending_state(&self) -> PendingState {
        self.pending.state()
    }

    pub(crate) fn pending_mut(&mut self) -> &mut PendingStateView<'a> {
        &mut self.pending
    }
}

// `rocksdb_transaction`/`into_parts` only exist on the transaction-backed instantiation
// (the writer uses them for commit/rollback/drop). A snapshot-backed read view has no transaction.
impl<'a, TAddr> RocksDbStateStoreReadTransaction<'a, TAddr, ReadOnlyTransaction<'a>> {
    pub(crate) fn rocksdb_transaction(&self) -> &Transaction<'a, TransactionDB> {
        &self.tx.inner
    }

    pub(crate) fn into_parts(self) -> (Transaction<'a, TransactionDB>, PendingStateView<'a>) {
        (self.tx.inner, self.pending)
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

    /// Every transaction that has a lock conflict recorded against it in any block.
    fn lock_conflicted_transaction_ids(&self) -> Result<HashSet<TransactionId>, RocksDbStorageError> {
        self.db()
            .cf(lock_conflict::LockConflictCf)?
            .key_iterator(Ordering::default(), "lock_conflicted_transaction_ids")
            .map(|res| res.map(|(transaction_id, _, _)| transaction_id))
            .collect()
    }

    /// Reads the pending chain ending at `leaf` in one walk of the pending-chain index.
    pub(super) fn read_pending_chain(&self, leaf: &BlockId) -> Result<PendingChain, RocksDbStorageError> {
        const OPERATION: &str = "read_pending_chain";
        let chain_cf = self.db().cf(chain::PendingChainIndex)?;

        let mut walk = Vec::new();
        if chain_cf.exists(leaf, OPERATION)? {
            walk.push(*leaf);
            let mut block_id = *leaf;
            while let Some(parent_id) = chain_cf.get(&block_id, OPERATION).optional()? {
                walk.push(parent_id);
                if parent_id.is_zero() {
                    break;
                }
                block_id = parent_id;
            }
        }

        let commit = self
            .get_commit_block()
            .optional()?
            .map(|commit| (commit.block_id, commit.epoch, commit.height));
        Ok(PendingChain::from_walk(*leaf, walk, commit))
    }

    /// Panics in debug builds if `chain` differs from the pending chain its leaf has in this transaction now, which is
    /// what reading it before a write to the pending-chain index and using it after would cause.
    fn debug_assert_chain_is_current(&self, chain: &PendingChain) -> Result<(), RocksDbStorageError> {
        if !cfg!(debug_assertions) {
            return Ok(());
        }
        let until = self.get_pending_chain_until(chain.leaf())?;
        assert_eq!(
            &until,
            chain.ancestry(),
            "pending chain for {} is stale: its ancestry differs from the pending-chain index",
            chain.leaf()
        );
        if chain.commit_position().is_some() {
            let ordered = self.get_pending_chain_ordered(chain.leaf())?;
            assert_eq!(
                ordered,
                chain.blocks(),
                "pending chain for {} is stale: its blocks differ from the pending-chain index",
                chain.leaf()
            );
        }
        Ok(())
    }

    /// The blocks whose rows belong to the chain ending at `leaf_block`.
    ///
    /// A lock, and anything else a block writes and a later block reads back, is a property of one branch. Rows written
    /// by a block on another branch must not answer a question asked about this one.
    pub(super) fn chain_scope(&self, leaf_block: &BlockId) -> Result<PendingChain, RocksDbStorageError> {
        let chain = self.read_pending_chain(leaf_block)?;
        self.require_commit_position(&chain)?;
        Ok(chain)
    }

    fn require_commit_position(&self, chain: &PendingChain) -> Result<(Epoch, NodeHeight), RocksDbStorageError> {
        match chain.commit_position() {
            Some(position) => Ok(position),
            // Raises the missing commit block as the error the direct read gives.
            None => {
                let commit_block = self.get_commit_block()?;
                Ok((commit_block.epoch, commit_block.height))
            },
        }
    }

    /// Whether `block` is on the chain `scope` ends in.
    ///
    /// Committing a block removes it from the pending chain, and every pending block sits above the commit block. So a
    /// block at or below the commit block's (epoch, height) that has left the pending chain was committed by the chain
    /// that committed that position, while a branch block still lingering there was not.
    pub(super) fn is_in_chain_scope(
        &self,
        scope: &PendingChain,
        block: &BlockId,
        epoch: Epoch,
        height: NodeHeight,
    ) -> Result<bool, RocksDbStorageError> {
        const OPERATION: &str = "is_in_chain_scope";
        if scope.contains_pending(block) {
            return Ok(true);
        }
        if (epoch, height) > self.require_commit_position(scope)? {
            return Ok(false);
        }
        Ok(!self.db().cf(chain::PendingChainIndex)?.exists(block, OPERATION)?)
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

    /// The last change to `substate_id` on the branch ending at the chain's leaf, or `None` if the branch contains no
    /// change for it. If `version` is given, only changes to that version are considered.
    ///
    /// Changes recorded by blocks that are not in the branch (forked-out siblings, other subtrees) are never
    /// considered. A substate version is only ever DOWNed after it is UPed, so a DOWN supersedes the UP of the same
    /// version.
    fn block_diffs_select_last_change(
        &self,
        chain: &PendingChain,
        substate_id: &SubstateId,
        version: Option<SubstateVersion>,
    ) -> Option<SubstateChange> {
        let pending = self.pending_state();
        let mut last_change = None::<&SubstateChange>;
        for block_id in chain.ancestry() {
            let Some(entry) = pending.block_diffs.get(block_id) else {
                continue;
            };
            let change = match version {
                Some(version) => entry.version_change(substate_id, version),
                None => entry.last_change(substate_id),
            };
            let Some(change) = change else {
                continue;
            };
            if last_change.is_none_or(|c| change_order(c) < change_order(change)) {
                last_change = Some(change);
            }
        }
        last_change.cloned()
    }

    /// Fails with a query error if the chain's leaf block does not exist. A leaf in the pending-chain index always
    /// does, so only an empty chain needs the lookup.
    fn require_leaf_block_exists(&self, chain: &PendingChain, operation: &str) -> Result<(), StorageError> {
        if chain.is_empty() && !self.blocks_exists(chain.leaf())? {
            return Err(StorageError::QueryError {
                reason: format!("{operation}: Block {} does not exist", chain.leaf()),
            });
        }
        Ok(())
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

    fn last_committed_epoch_end_get(&self) -> Result<LeafBlock, StorageError> {
        Ok(self
            .db()
            .cf(LastCommittedEpochEndCf)?
            .get_by_default_key("last_committed_epoch_end_get")?)
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
        let proposed_in_block_query = self.db().cf(foreign_proposal::ByProposedInBlockIndexQuery)?;
        let mut proposed_in_chain = HashSet::new();
        for pending_block_id in &pending_block_ids {
            for result in proposed_in_block_query.query_prefix_range_key_iterator(Ordering::default(), pending_block_id)
            {
                let (_, fp_block_id) = result?;
                proposed_in_chain.insert(fp_block_id);
            }
        }
        let iter = unconfirmed_cf.key_iterator(Ordering::Ascending, OPERATION);

        let mut proposals = vec![];

        for result in iter {
            let (epoch, block_id) = result?;
            // Since the iterator is ordered by epoch, we're done since
            // we don't want to propose FPs from future epochs until we've progressed to that epoch
            if epoch > locked.epoch {
                break;
            }

            if proposed_in_chain.contains(&block_id) {
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
        let Some(key) = iter.next().transpose()? else {
            return Err(StorageError::NotFound {
                item: "TransactionExecution",
                key: format!("{tx_id}"),
            });
        };
        let execution = self.db().cf(BlockTransactionExecutionCf)?.get(&key, OPERATION)?;

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
                &(
                    *transaction_id,
                    *from_block.block_id(),
                    from_block.epoch(),
                    from_block.height(),
                ),
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

        // Heights restart at zero each epoch, so the most recent ancestor is the one with the highest (epoch, height).
        let mut best: Option<(TransactionId, BlockId, Epoch, NodeHeight)> = None;
        let candidates = query.query_prefix_range_key_iterator(Ordering::default(), transaction_id);
        for res in candidates {
            let (tx_id, block_id, epoch, height) = res?;
            let is_ancestor = pending_chain.contains(&block_id) || !pending_index.exists(&block_id, OPERATION)?;
            if is_ancestor && best.as_ref().is_none_or(|(_, _, e, h)| (*e, *h) < (epoch, height)) {
                best = Some((tx_id, block_id, epoch, height));
            }
        }

        if let Some(key) = best {
            let (_, block_id, epoch, height) = key;
            debug!(
                target: LOG_TARGET,
                "{OPERATION}: Found execution for {transaction_id} in {block_id} {epoch}/{height}",
            );
            let execution = cf.get(&key, OPERATION)?;
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
            let (block_id, tx_id, epoch, height) = result?;
            let execution = cf.get(&(tx_id, block_id, epoch, height), OPERATION)?;
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
        let changes = self
            .pending_state()
            .block_diffs
            .get(block_id)
            .map(|entry| entry.changes().to_vec())
            .unwrap_or_default();
        Ok(BlockDiff::new(*block_id, changes))
    }

    fn pending_chain_get(&self, leaf: &BlockId) -> Result<PendingChain, StorageError> {
        Ok(self.read_pending_chain(leaf)?)
    }

    fn block_diffs_get_last_change_for_substate_in_chain(
        &self,
        chain: &PendingChain,
        substate_id: &SubstateId,
    ) -> Result<SubstateChange, StorageError> {
        const OPERATION: &str = "block_diffs_get_last_change_for_substate";
        self.debug_assert_chain_is_current(chain)?;
        self.require_leaf_block_exists(chain, OPERATION)?;

        self.block_diffs_select_last_change(chain, substate_id, None)
            .ok_or_else(|| StorageError::NotFound {
                item: "SubstateChange",
                key: format!("{substate_id} in {}", chain.leaf()),
            })
    }

    fn block_diffs_get_change_for_versioned_substate_in_chain<'a, T: Into<VersionedSubstateIdRef<'a>>>(
        &self,
        chain: &PendingChain,
        substate_id: T,
    ) -> Result<SubstateChange, StorageError> {
        const OPERATION: &str = "block_diffs_get_change_for_versioned_substate";
        self.debug_assert_chain_is_current(chain)?;
        self.require_leaf_block_exists(chain, OPERATION)?;

        let versioned = substate_id.into();

        self.block_diffs_select_last_change(chain, versioned.substate_id(), Some(versioned.version()))
            .ok_or_else(|| StorageError::NotFound {
                item: "SubstateChange",
                key: format!("{versioned} in {}", chain.leaf()),
            })
    }

    fn block_diffs_contains_versioned_substate_in_chain<'a, T: Into<VersionedSubstateIdRef<'a>>>(
        &self,
        chain: &PendingChain,
        substate_id: T,
    ) -> Result<bool, StorageError> {
        const OPERATION: &str = "block_diffs_contains_versioned_substate";
        self.debug_assert_chain_is_current(chain)?;
        self.require_leaf_block_exists(chain, OPERATION)?;

        let versioned = substate_id.into();

        let pending = self.pending_state();
        let found = chain.ancestry().iter().any(|block_id| {
            pending.block_diffs.get(block_id).is_some_and(|entry| {
                entry
                    .version_change(versioned.substate_id(), versioned.version())
                    .is_some()
            })
        });
        Ok(found)
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
        const OPERATION: &str = "proposal_certificates_get_many";
        let qcs = self
            .db()
            .cf(ProposalCertificateCf)?
            .multi_get_exact(qc_ids, OPERATION)?;
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
        let tcs = self.db().cf(TimeoutCertificateCf)?.multi_get_exact(ids, OPERATION)?;
        Ok(tcs)
    }

    fn transaction_pool_get_for_blocks_in_chain(
        &self,
        chain: &PendingChain,
        transaction_id: &TransactionId,
    ) -> Result<TransactionPoolRecord, StorageError> {
        const OPERATION: &str = "transaction_pool_get_for_blocks";
        self.debug_assert_chain_is_current(chain)?;
        self.require_leaf_block_exists(chain, OPERATION)?;

        let cf = self.db().cf(TransactionPoolCf)?;
        let query = self
            .db()
            .cf(transaction_pool_state_update::TransactionPoolStateUpdateCf)?;

        let mut transaction = cf.get(transaction_id, OPERATION)?;

        trace!(
            target: LOG_TARGET,
            "{OPERATION}: pending_chain: {} for block {}",
            chain.blocks().display(), chain.leaf()
        );

        for block_id in chain.blocks() {
            if let Some(update) = query.get(&(*block_id, *transaction_id), OPERATION).optional()? {
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

        let iter = cf.value_iterator(Ordering::Ascending, OPERATION);
        let mut transactions = Vec::new();
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

    fn transaction_pool_for_each_ready<F>(&self, block_id: &BlockId, mut f: F) -> Result<(), StorageError>
    where F: FnMut(TransactionPoolRecord) {
        const OPERATION: &str = "transaction_pool_for_each_ready";
        if !self.blocks_exists(block_id)? {
            return Err(StorageError::QueryError {
                reason: format!("transaction_pool_get_for_blocks: Block {} does not exist", block_id),
            });
        }

        let cf = self.db().cf(TransactionPoolCf)?;

        let query = self.db().cf(transaction_pool_state_update::ByBlockIdQuery)?;
        let lock_conflicted = self.lock_conflicted_transaction_ids()?;

        let pending_chain = self.get_pending_chain_ordered(block_id)?;

        let mut updates = HashMap::new();
        for block_id in pending_chain.into_iter().rev() {
            let iter = query.query_prefix_range_iterator(Ordering::default(), &block_id);
            for result in iter {
                let ((_, tx_id), update) = result?;
                if lock_conflicted.contains(&tx_id) {
                    continue;
                }
                updates.insert(tx_id, update);
            }
        }

        let iter = cf.value_iterator(Ordering::default(), OPERATION);

        for result in iter {
            let mut tx = result?;
            if lock_conflicted.contains(tx.id()) {
                continue;
            }
            if let Some(update) = updates.remove(tx.id()) {
                update.merge_into(&mut tx);
            }
            if tx.is_ready() {
                f(tx);
            }
        }

        Ok(())
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
        if stage.is_none() && is_ready.is_none() && !skip_lock_conflicted {
            return Ok(cf.count(OPERATION)?);
        }

        let lock_conflicted = skip_lock_conflicted
            .then(|| self.lock_conflicted_transaction_ids())
            .transpose()?
            .unwrap_or_default();

        let mut count = 0;
        for result in cf.value_iterator(Ordering::default(), OPERATION) {
            let rec = result?;
            if stage.is_some_and(|stage| rec.current_stage() != stage) ||
                is_ready.is_some_and(|is_ready| rec.is_ready() != is_ready) ||
                lock_conflicted.contains(rec.id())
            {
                continue;
            }
            count += 1;
        }

        Ok(count)
    }

    fn transaction_pool_any<F>(&self, skip_lock_conflicted: bool, mut predicate: F) -> Result<bool, StorageError>
    where F: FnMut(&TransactionPoolRecord) -> bool {
        const OPERATION: &str = "transaction_pool_any";

        let lock_conflicted = skip_lock_conflicted
            .then(|| self.lock_conflicted_transaction_ids())
            .transpose()?
            .unwrap_or_default();

        let cf = self.db().cf(TransactionPoolCf)?;
        for result in cf.value_iterator(Ordering::default(), OPERATION) {
            let rec = result?;
            if !lock_conflicted.contains(rec.id()) && predicate(&rec) {
                return Ok(true);
            }
        }

        Ok(false)
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

    fn substates_get_max_version_for_substate(
        &self,
        substate_id: &SubstateId,
    ) -> Result<(SubstateVersion, bool), StorageError> {
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
        leaf_block: &LeafBlock,
        transaction_id: &TransactionId,
    ) -> Result<Vec<LockedSubstateValue>, StorageError> {
        const OPERATION: &str = "substate_locks_get_locked_substates_for_transaction";

        let scope = self.chain_scope(leaf_block.block_id())?;
        let pending = self.pending_state();
        let table = &pending.locks;

        // Ordered by encoded substate id, then by block and grant order, so callers see one order on every node.
        let mut held = Vec::new();
        for (block_id, block, substate_id, grant_seq, lock) in table.locks_held_by(transaction_id) {
            if !self.is_in_chain_scope(&scope, block_id, block.epoch, block.height)? {
                continue;
            }
            let sort_key = (
                SubstateIdCodec.encode(substate_id)?.to_vec(),
                *block_id,
                block.epoch,
                block.height,
                grant_seq,
            );
            held.push((sort_key, substate_id, *lock));
        }
        held.sort_by(|(a, ..), (b, ..)| a.cmp(b));

        let substates_cf = self.db().cf(SubstateCf)?;
        let mut locked_substates = Vec::with_capacity(held.len());
        for (_, substate_id, lock) in held {
            let substate = substates_cf
                .get(
                    &SubstateAddress::from_substate_id(substate_id, lock.version()),
                    OPERATION,
                )
                .optional()?;
            locked_substates.push(LockedSubstateValue {
                substate_id: substate_id.clone(),
                lock,
                value: substate.and_then(|s| s.into_substate_value()),
            });
        }

        Ok(locked_substates)
    }

    /// Returns the transaction ID of any write lock found for any of the given substates, or None if there are no write
    /// locks. Of several transactions write-locking one substate, the lowest id is returned.
    ///
    /// # Used for:
    /// Foreign proposal conflict resolution, to check if there is a conflicting write lock in a foreign proposal by
    /// another locally proposed transaction.
    fn substate_locks_has_any_write_locks_for_substates<'a, I: IntoIterator<Item = &'a SubstateId>>(
        &self,
        leaf_block: &LeafBlock,
        exclude_transaction_id: Option<&TransactionId>,
        substate_ids: I,
    ) -> Result<Option<TransactionId>, StorageError> {
        let mut substate_ids = substate_ids.into_iter().peekable();
        if substate_ids.peek().is_none() {
            return Ok(None);
        }

        let scope = self.chain_scope(leaf_block.block_id())?;
        let pending = self.pending_state();
        let table = &pending.locks;

        for substate_id in substate_ids {
            let mut conflicting = None::<TransactionId>;
            for (block_id, block, locks) in table.blocks_locking_substate(substate_id) {
                let lowest = locks
                    .iter()
                    .filter(|lock| lock.is_write())
                    .map(|lock| *lock.transaction_id())
                    .filter(|id| exclude_transaction_id != Some(id))
                    .min();
                let Some(lowest) = lowest else {
                    continue;
                };
                if conflicting.is_some_and(|c| c <= lowest) {
                    continue;
                }
                if self.is_in_chain_scope(&scope, block_id, block.epoch, block.height)? {
                    conflicting = Some(lowest);
                }
            }
            if conflicting.is_some() {
                return Ok(conflicting);
            }
        }

        Ok(None)
    }

    /// Returns the lock most recently granted on a substate by the chain ending at the chain's leaf, searching its
    /// pending blocks and then the committed chain beneath them.
    ///
    /// The answer is a property of that chain alone: locks granted by blocks on other branches are skipped. Heights
    /// restart at zero each epoch and a chain holds one block per height within an epoch, so (block_epoch,
    /// block_height, grant order) totally orders the locks any one chain holds on the substate: the latest lock of the
    /// highest block in the chain is the answer.
    ///
    /// # Used for:
    /// Local proposal conflict resolution, to check if a substate is locked by another transaction.
    fn substate_locks_get_latest_for_substate_in_chain(
        &self,
        scope: &PendingChain,
        substate_id: &SubstateId,
    ) -> Result<SubstateLock, StorageError> {
        self.debug_assert_chain_is_current(scope)?;
        let pending = self.pending_state();
        let table = &pending.locks;

        let mut candidates = table.blocks_locking_substate(substate_id).collect::<Vec<_>>();
        candidates.sort_by(|(a_id, a, _), (b_id, b, _)| (b.epoch, b.height, b_id).cmp(&(a.epoch, a.height, a_id)));
        for (block_id, block, locks) in candidates {
            if !self.is_in_chain_scope(scope, block_id, block.epoch, block.height)? {
                continue;
            }
            if let Some(lock) = locks.last() {
                return Ok(*lock);
            }
        }

        Err(StorageError::NotFound {
            item: "SubstateLock",
            key: format!("for substate {substate_id} in block {}", scope.leaf()),
        })
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

        let pending = self.pending_state();
        let mut diffs = HashMap::<Shard, Vec<PendingShardStateTreeDiff>>::new();
        // Earliest to latest
        for block_id in block_ids.iter().rev() {
            let Some(record) = pending.tree_diffs.get(block_id) else {
                continue;
            };
            for ShardStateTreeDiff { shard, diff } in record.iter() {
                trace!(
                    target: LOG_TARGET,
                    "{OPERATION}: got diff for shard {} in block {} (v{}, new={}, stale={})",
                    shard, block_id, diff.version, diff.diff.new_nodes.len(), diff.diff.stale_tree_nodes.len()
                );
                diffs.entry(*shard).or_default().push(diff.clone());
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
        let substates = substate_cf.multi_get_exact(data.transitions.iter().map(|t| t.substate_address), OPERATION)?;

        let mut updates = Vec::with_capacity(data.transitions.len());
        let all_hashes = value_filter.include_filtered_hashes();
        let up_only = value_filter.is_up_only();
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

    fn state_sync_rewind_point_get(&self, shard: Shard) -> Result<Option<Version>, StorageError> {
        const OPERATION: &str = "state_sync_rewind_point_get";
        let version = self
            .db()
            .cf(StateSyncRewindPointCf)?
            .get(&shard, OPERATION)
            .optional()?;
        Ok(version)
    }

    fn state_version_proofs_get_range(
        &self,
        shard: Shard,
        from: Version,
        to: Version,
    ) -> Result<Vec<StateVersionProof>, StorageError> {
        let cf = self.db().cf(StateVersionProofCf)?;
        let start = cf.encode_key(&(shard, from));
        let end = cf.encode_key(&(shard, to.saturating_add(1)));
        let proofs = cf
            .range_iterator(Ordering::Ascending, start..end)
            .map(|res| res.map(|(_, proof)| proof))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(proofs)
    }

    fn block_commit_proofs_get(&self, block_id: &BlockId) -> Result<Vec<u8>, StorageError> {
        const OPERATION: &str = "block_commit_proofs_get";
        let commit_proof = self.db().cf(BlockCommitProofCf)?.get(block_id, OPERATION)?;
        Ok(commit_proof)
    }

    fn substate_down_proofs_get(
        &self,
        shard: Shard,
        id: &VersionedSubstateId,
    ) -> Result<Option<SubstateDownProofRecord>, StorageError> {
        const OPERATION: &str = "substate_down_proofs_get";
        let record = self
            .db()
            .cf(SubstateDownProofCf)?
            .get(&(shard, id.to_substate_address()), OPERATION)
            .optional()?;
        Ok(record)
    }

    fn state_sync_rewind_points_get_all(&self) -> Result<Vec<(Shard, Version)>, StorageError> {
        const OPERATION: &str = "state_sync_rewind_points_get_all";
        let points = self
            .db()
            .cf(StateSyncRewindPointCf)?
            .iterator(Ordering::Ascending, OPERATION)
            .collect::<Result<_, _>>()?;
        Ok(points)
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

    fn epoch_checkpoint_get_all_for_epoch(&self, epoch: Epoch) -> Result<Vec<EpochCheckpoint>, StorageError> {
        let query = self.db().cf(epoch_checkpoint::ByEpochQuery)?;
        let iter = query.query_prefix_range_iterator(Ordering::Ascending, &epoch);
        let epoch_checkpoints = iter
            .map(|result| result.map(|(_, checkpoint)| checkpoint))
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

    fn validator_liveness_counters_as_of(
        &self,
        epoch: Epoch,
        public_key: &RistrettoPublicKeyBytes,
        as_of: NodeHeight,
    ) -> Result<Option<LivenessCounters>, StorageError> {
        let cf = self.db().cf(ValidatorLivenessLogCf)?;
        let start = cf.encode_key(&(epoch, *public_key, NodeHeight::zero()));
        // The upper bound is exclusive and the query is inclusive of `as_of`.
        let end = cf.encode_key(&(epoch, *public_key, as_of.saturating_add(NodeHeight(1))));
        let counters = cf
            .range_iterator(Ordering::Descending, start..end)
            .next()
            .transpose()?
            .map(|(_, counters)| counters);
        Ok(counters)
    }

    fn vote_equivocation_exists(
        &self,
        epoch: Epoch,
        height: NodeHeight,
        public_key: &RistrettoPublicKeyBytes,
    ) -> Result<bool, StorageError> {
        const OPERATION: &str = "vote_equivocation_exists";
        let exists = self
            .db()
            .cf(vote_equivocation::VoteEquivocationCf)?
            .exists(&(epoch, height, *public_key), OPERATION)?;
        Ok(exists)
    }

    fn vote_equivocation_exists_for_validator(
        &self,
        epoch: Epoch,
        public_key: &RistrettoPublicKeyBytes,
    ) -> Result<bool, StorageError> {
        // The evidence is keyed by view before signer, so this scans the epoch. Only one record per
        // view and signer is kept and equivocation is rare, so there is normally nothing to scan.
        for key in self
            .db()
            .cf(vote_equivocation::ByEpochQuery)?
            .query_prefix_range_key_iterator(Ordering::Ascending, &epoch)
        {
            let (_, _, signer) = key?;
            if signer == *public_key {
                return Ok(true);
            }
        }

        Ok(false)
    }
}
