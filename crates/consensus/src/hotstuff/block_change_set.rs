//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, hash_map::Entry},
    fmt::{Display, Formatter},
    ops::Deref,
};

use indexmap::IndexMap;
use log::*;
use tari_consensus_types::{BlockId, HighPc, HighTc, LeafBlock};
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{NodeHeight, ShardGroup, displayable::Displayable, optional::Optional, shard::Shard};
use tari_ootle_storage::{
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{
        Block,
        BlockDiff,
        BlockTransactionExecution,
        Evidence,
        ForeignProposalRecord,
        ForeignProposalStatus,
        NoVoteReason,
        PendingShardStateTreeDiff,
        SubstateChange,
        SubstateLock,
        SubstatePledge,
        SubstatePledges,
        SubstateRecord,
        TransactionExecution,
        TransactionPoolError,
        TransactionPoolRecord,
        TransactionPoolStatusUpdate,
    },
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_sidechain::QuorumDecision;
use tari_template_lib_types::ClaimedOutputTombstoneAddress;

use crate::{hotstuff::transaction_manager::TransactionLockConflicts, tracing::TraceTimer};

const LOG_TARGET: &str = "tari::ootle::consensus::block_change_set";

const MEM_MAX_BLOCK_DIFF_CHANGES: usize = 10000;
const MEM_MAX_STATE_TREE_DIFF_SIZE: usize = 1000;
const MEM_MAX_SUBSTATE_LOCK_SIZE: usize = 100000;
const MEM_MAX_TRANSACTION_CHANGE_SIZE: usize = 1000;
const MEM_NEW_SEQUENCE_TRANSACTIONS: usize = 1000;
const MEM_MAX_PROPOSED_FOREIGN_PROPOSALS_SIZE: usize = 1000;
const MEM_MAX_PROPOSED_UTXO_MINTS_SIZE: usize = 1000;

#[derive(Debug, Clone)]
pub struct BlockDecision {
    pub local_decision: Option<QuorumDecision>,
    /// Contains newly-committed non-dummy blocks
    pub commit_blocks: Vec<Block>,
    pub finalized_transactions: Vec<Vec<TransactionPoolRecord>>,
    pub high_pc: HighPc,
    pub new_high_tc: Option<HighTc>,
    pub no_vote_reason: Option<NoVoteReason>,
}

impl BlockDecision {
    pub fn is_accept(&self) -> bool {
        matches!(self.local_decision, Some(QuorumDecision::Accept))
    }

    pub fn is_committed_epoch_end(&self) -> bool {
        self.commit_blocks.iter().any(|block| block.is_epoch_end())
    }

    pub fn take_end_of_epoch_block(&mut self) -> Option<Block> {
        if let Some(pos) = self.commit_blocks.iter().position(|block| block.is_epoch_end()) {
            let block = self.commit_blocks.remove(pos);
            Some(block)
        } else {
            None
        }
    }

    pub fn highest_qc_view(&self) -> NodeHeight {
        self.high_pc
            .height()
            .max(self.new_high_tc.as_ref().map_or(NodeHeight(0), |tc| tc.height()))
    }
}

#[derive(Debug, Clone)]
pub struct ProposedBlockChangeSet {
    block: LeafBlock,
    quorum_decision: Option<QuorumDecision>,
    local_substate_changes: Vec<SubstateChange>,
    state_tree_diffs: IndexMap<Shard, PendingShardStateTreeDiff>,
    substate_locks: IndexMap<SubstateId, Vec<SubstateLock>>,
    transaction_changes: IndexMap<TransactionId, TransactionChangeSet>,
    new_transactions_to_sequence: Vec<TransactionPoolRecord>,
    proposed_foreign_proposals: Vec<BlockId>,
    proposed_utxo_mints: Vec<ClaimedOutputTombstoneAddress>,
    no_vote_reason: Option<NoVoteReason>,
}

impl ProposedBlockChangeSet {
    pub fn new(block: LeafBlock) -> Self {
        Self {
            block,
            quorum_decision: None,
            local_substate_changes: Vec::new(),
            substate_locks: IndexMap::new(),
            transaction_changes: IndexMap::new(),
            new_transactions_to_sequence: Vec::new(),
            state_tree_diffs: IndexMap::new(),
            proposed_foreign_proposals: Vec::new(),
            proposed_utxo_mints: Vec::new(),
            no_vote_reason: None,
        }
    }

    pub fn set_block(&mut self, block: LeafBlock) -> &mut Self {
        self.block = block;
        self
    }

    pub fn set_no_vote(&mut self, no_vote_reason: NoVoteReason) -> &mut Self {
        self.no_vote_reason = Some(no_vote_reason);
        self
    }

    pub fn no_vote_reason(&self) -> Option<&NoVoteReason> {
        self.no_vote_reason.as_ref()
    }

    pub fn clear(&mut self) {
        self.quorum_decision = None;

        self.local_substate_changes.clear();
        if self.local_substate_changes.capacity() > MEM_MAX_BLOCK_DIFF_CHANGES {
            debug!(
                target: LOG_TARGET,
                "Shrinking block_diff from {} to {}",
                self.local_substate_changes.capacity(),
                MEM_MAX_BLOCK_DIFF_CHANGES
            );
            self.local_substate_changes.shrink_to(MEM_MAX_BLOCK_DIFF_CHANGES);
        }
        self.transaction_changes.clear();
        if self.transaction_changes.capacity() > MEM_MAX_TRANSACTION_CHANGE_SIZE {
            debug!(
                target: LOG_TARGET,
                "Shrinking transaction_changes from {} to {}",
                self.transaction_changes.capacity(),
                MEM_MAX_TRANSACTION_CHANGE_SIZE
            );
            self.transaction_changes.shrink_to(MEM_MAX_TRANSACTION_CHANGE_SIZE);
        }
        self.new_transactions_to_sequence.clear();
        if self.new_transactions_to_sequence.capacity() > MEM_NEW_SEQUENCE_TRANSACTIONS {
            debug!(
                target: LOG_TARGET,
                "Shrinking new_transactions_to_sequence from {} to {}",
                self.new_transactions_to_sequence.capacity(),
                MEM_NEW_SEQUENCE_TRANSACTIONS
            );
            self.new_transactions_to_sequence
                .shrink_to(MEM_NEW_SEQUENCE_TRANSACTIONS);
        }
        self.state_tree_diffs.clear();
        if self.state_tree_diffs.capacity() > MEM_MAX_STATE_TREE_DIFF_SIZE {
            debug!(
                target: LOG_TARGET,
                "Shrinking state_tree_diffs from {} to {}",
                self.state_tree_diffs.capacity(),
                MEM_MAX_STATE_TREE_DIFF_SIZE
            );
            self.state_tree_diffs.shrink_to(MEM_MAX_STATE_TREE_DIFF_SIZE);
        }
        self.substate_locks.clear();
        if self.substate_locks.capacity() > MEM_MAX_SUBSTATE_LOCK_SIZE {
            debug!(
                target: LOG_TARGET,
                "Shrinking substate_locks from {} to {}",
                self.substate_locks.capacity(),
                MEM_MAX_SUBSTATE_LOCK_SIZE
            );
            self.substate_locks.shrink_to(MEM_MAX_SUBSTATE_LOCK_SIZE);
        }
        self.proposed_foreign_proposals.clear();
        if self.proposed_foreign_proposals.capacity() > MEM_MAX_PROPOSED_FOREIGN_PROPOSALS_SIZE {
            debug!(
                target: LOG_TARGET,
                "Shrinking proposed_foreign_proposals from {} to {}",
                self.proposed_foreign_proposals.capacity(),
                MEM_MAX_PROPOSED_FOREIGN_PROPOSALS_SIZE
            );
            self.proposed_foreign_proposals
                .shrink_to(MEM_MAX_PROPOSED_FOREIGN_PROPOSALS_SIZE);
        }
        self.proposed_utxo_mints.clear();
        if self.proposed_utxo_mints.capacity() > MEM_MAX_PROPOSED_UTXO_MINTS_SIZE {
            debug!(
                target: LOG_TARGET,
                "Shrinking proposed_utxo_mints from {} to {}",
                self.proposed_utxo_mints.capacity(),
                MEM_MAX_PROPOSED_UTXO_MINTS_SIZE
            );
            self.proposed_utxo_mints.shrink_to(MEM_MAX_PROPOSED_UTXO_MINTS_SIZE);
        }
        self.no_vote_reason = None;
    }

    pub fn set_state_tree_diffs(&mut self, diffs: IndexMap<Shard, PendingShardStateTreeDiff>) -> &mut Self {
        self.state_tree_diffs = diffs;
        self
    }

    pub fn set_quorum_decision(&mut self, decision: QuorumDecision) -> &mut Self {
        self.quorum_decision = Some(decision);
        self
    }

    pub fn set_block_diff_to_commit(&mut self, diff: BlockDiff) -> &mut Self {
        self.local_substate_changes = diff.into_changes();
        self
    }

    pub fn set_substate_locks(&mut self, locks: IndexMap<SubstateId, Vec<SubstateLock>>) -> &mut Self {
        self.substate_locks = locks;
        self
    }

    pub fn set_foreign_proposal_proposed_in(&mut self, foreign_proposal_block_id: BlockId) -> &mut Self {
        self.proposed_foreign_proposals.push(foreign_proposal_block_id);
        self
    }

    pub fn proposed_foreign_proposals(&self) -> &[BlockId] {
        &self.proposed_foreign_proposals
    }

    pub fn set_utxo_mint_proposed_in(&mut self, mint: ClaimedOutputTombstoneAddress) -> &mut Self {
        self.proposed_utxo_mints.push(mint);
        self
    }

    pub fn apply_transaction_update(&self, tx_rec_mut: &mut TransactionPoolRecord) {
        if let Some(update) = self.transaction_changes.get(tx_rec_mut.id()) {
            update.apply_update(tx_rec_mut);
        }
    }

    pub fn add_foreign_pledges(
        &mut self,
        transaction_id: &TransactionId,
        shard_group: ShardGroup,
        foreign_pledges: SubstatePledges,
    ) -> &mut Self {
        let change_mut = self.transaction_changes.entry(*transaction_id).or_default();
        match change_mut.foreign_pledges.entry(shard_group) {
            Entry::Vacant(entry) => {
                entry.insert(foreign_pledges);
            },
            Entry::Occupied(mut entry) => {
                // Multiple foreign pledges for the same shard group included the block
                // This can happen if a LocalPrepare and LocalAccept are proposed for the same transaction in the same
                // block
                entry.get_mut().extend(foreign_pledges);
            },
        }
        self
    }

    pub fn get_foreign_pledges(&self, transaction_id: &TransactionId) -> impl Iterator<Item = &SubstatePledge> + Clone {
        self.transaction_changes
            .get(transaction_id)
            .into_iter()
            .flat_map(|change| change.foreign_pledges.values())
            .flatten()
    }

    pub fn is_accept(&self) -> bool {
        matches!(self.quorum_decision, Some(QuorumDecision::Accept))
    }

    pub fn quorum_decision(&self) -> Option<QuorumDecision> {
        self.quorum_decision
    }

    pub fn transaction_execution(&self, transaction_id: &TransactionId) -> Option<&BlockTransactionExecution> {
        self.transaction_changes
            .get(transaction_id)
            .and_then(|change| change.execution.as_ref())
    }

    pub fn take_transaction_execution(&mut self, transaction_id: &TransactionId) -> Option<BlockTransactionExecution> {
        self.transaction_changes
            .get_mut(transaction_id)
            .and_then(|change| change.execution.take())
    }

    /// Removes the execution recorded for each transaction and yields it.
    ///
    /// The rest of each transaction's change — the pending pool update, foreign pledges, evidence — stays in
    /// the change set. Those describe state that later commands in the same block are evaluated against, so
    /// they must outlive the executions that were produced alongside them.
    pub fn take_transaction_executions(&mut self) -> Vec<(TransactionId, TransactionExecution)> {
        self.transaction_changes
            .iter_mut()
            .filter_map(|(tx_id, change)| {
                change
                    .execution
                    .take()
                    .map(|execution| (*tx_id, execution.into_transaction_execution()))
            })
            .collect()
    }

    pub fn add_transaction_execution(
        &mut self,
        transaction_id: TransactionId,
        execution: TransactionExecution,
    ) -> Result<&mut Self, TransactionPoolError> {
        let execution = execution.for_block(self.block, transaction_id);
        let change_mut = self.transaction_changes.entry(*execution.transaction_id()).or_default();
        if change_mut.execution.is_some() {
            return Err(TransactionPoolError::TransactionAlreadyExecuted {
                transaction_id: *execution.transaction_id(),
                block_id: *self.block.block_id(),
            });
        }

        change_mut.execution = Some(execution);
        Ok(self)
    }

    pub fn get_transaction_pool_record<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        leaf_block: &LeafBlock,
        transaction_id: &TransactionId,
    ) -> Result<TransactionPoolRecord, TransactionPoolError> {
        let rec = self
            .transaction_changes
            .get(transaction_id)
            .and_then(|change| {
                change.next_update.as_ref().map(|u| {
                    debug!(
                        target: LOG_TARGET,
                        "Found cached transaction update for {} in block {}",
                        transaction_id,
                        leaf_block.block_id()
                    );
                    u.transaction()
                })
            })
            .cloned()
            .map(Ok)
            .or_else(|| {
                TransactionPoolRecord::get(tx, leaf_block.block_id(), transaction_id)
                    .optional()
                    .inspect(|a| {
                        a.as_ref().inspect(|a| {
                            debug!(
                                target: LOG_TARGET,
                                "LOADED transaction pool record for {} in block {}: {:#}",
                                transaction_id,
                                leaf_block.block_id(),
                                a.evidence()
                            );
                        });
                    })
                    .transpose()
            })
            .transpose()?
            .ok_or_else(|| StorageError::NotFound {
                item: "TransactionPoolRecord",
                key: transaction_id.to_string(),
            })?;
        Ok(rec)
    }

    pub fn sequence_new_transaction(
        &mut self,
        transaction: &Transaction,
        initial_evidence: Evidence,
    ) -> TransactionPoolRecord {
        let rec = TransactionPoolRecord::new_from_transaction(transaction, initial_evidence);
        self.new_transactions_to_sequence.push(rec.clone());
        rec
    }

    pub fn set_next_transaction_update(
        &mut self,
        transaction: TransactionPoolRecord,
    ) -> Result<&mut Self, TransactionPoolError> {
        let change_mut = self.transaction_changes.entry(*transaction.id()).or_default();

        debug!(
            target: LOG_TARGET,
            "set_next_transaction_update: evidence: {}",
            transaction.evidence(),
        );
        let ready_now = transaction.is_ready_for_pending_stage(self.block.shard_group());
        let next_update = TransactionPoolStatusUpdate::new(transaction, ready_now);
        debug!(
            target: LOG_TARGET,
            "📝 next transaction update ({} {} {} is_ready_now={}, {:#}) in block {}",
            next_update.transaction().id(),
            next_update.transaction().current_stage(),
            next_update.decision(),
            next_update.is_ready_now(),
            next_update.transaction().evidence(),
            self.block
        );
        change_mut.next_update = Some(next_update);
        Ok(self)
    }
}

impl ProposedBlockChangeSet {
    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref,
        TTx::Target: StateStoreReadTransaction,
    {
        if let Some(ref reason) = self.no_vote_reason {
            warn!(target: LOG_TARGET, "❌ No vote: {}", reason);
            if let Err(err) = tx.diagnostics_add_no_vote(*self.block.block_id(), reason.clone()) {
                error!(target: LOG_TARGET, "Failed to save no vote reason: {}", err);
            }
            // No vote
            return Ok(());
        }

        let _timer = TraceTimer::debug(LOG_TARGET, "ProposedBlockChangeSet::save");
        // Store the block diff
        BlockDiff::insert(tx, &self.block.block_id, &self.local_substate_changes)?;

        // Store the tree diffs for each affected shard
        for (shard, diff) in &self.state_tree_diffs {
            PendingShardStateTreeDiff::create(tx, *self.block.block_id(), *shard, diff)?;
        }

        // Save locks
        SubstateRecord::lock_all(tx, &self.block, &self.substate_locks)?;

        // Sequence new transactions
        for pool_rec in &self.new_transactions_to_sequence {
            debug!(
                target: LOG_TARGET,
                "📝 Sequencing new transaction {} {} {} is_ready_now={}",
                pool_rec.id(),
                pool_rec.current_stage(),
                pool_rec.current_local_decision(),
                pool_rec.is_ready()
            );
            // TODO: TransactionPool abstraction is not great, so calling the store method directly
            tx.transaction_pool_insert_new(
                *pool_rec.id(),
                pool_rec.current_local_decision(),
                pool_rec.evidence(),
                pool_rec.is_ready(),
                pool_rec.is_global(),
                pool_rec.max_epoch(),
                pool_rec.transaction_weight(),
            )?;
        }

        for (transaction_id, change) in &self.transaction_changes {
            // Save any transaction executions for the block
            if let Some(ref execution) = change.execution {
                // This may already exist if we proposed the block
                if execution.insert_if_required(tx)? {
                    debug!(
                        target: LOG_TARGET,
                        "📝 Transaction execution for {} saved in block {}",
                        execution.transaction_id(),
                        self.block.block_id
                    );
                } else {
                    debug!(
                        target: LOG_TARGET,
                        "📝 Transaction execution for {} already exists in block {}",
                        execution.transaction_id(),
                        self.block.block_id
                    );
                }
            }

            // Save any transaction pool updates
            if let Some(ref update) = change.next_update {
                if update.decision().is_abort() {
                    // Remove lock conflicts for this transaction. This allows other transactions to be proposed.
                    TransactionLockConflicts::remove_for_transactions(tx, Some(update.transaction_id()))?;
                }
                update.insert_for_block(tx, &self.block)?;
            }

            for (shard_group, pledges) in &change.foreign_pledges {
                tx.foreign_substate_pledges_save(transaction_id, *shard_group, pledges)?;
            }
        }

        // TODO(perf): batch this update
        for block_id in &self.proposed_foreign_proposals {
            ForeignProposalRecord::set_status_by_id(
                tx,
                block_id,
                ForeignProposalStatus::Proposed,
                Some(self.block.block_id()),
            )?;
        }

        Ok(())
    }

    pub fn log_everything_dropped(&self) {
        if !log_enabled!(log::Level::Debug) {
            return;
        }
        const LOG_TARGET: &str = "tari::ootle::consensus::block_change_set::debug";
        debug!(target: LOG_TARGET, "❌ No vote: {}", self.no_vote_reason.display());
        let _timer = TraceTimer::debug(LOG_TARGET, "ProposedBlockChangeSet::save_for_debug");
        // TODO: consider persisting this data somewhere

        for change in &self.local_substate_changes {
            debug!(target: LOG_TARGET, "[drop] SubstateChange: {}", change);
        }

        // Store the tree diffs for each effected shard
        for (shard, diff) in &self.state_tree_diffs {
            debug!(target: LOG_TARGET, "[drop] StateTreeDiff: shard: {}, diff: {}", shard, diff);
        }

        for (substate_id, locks) in &self.substate_locks {
            debug!(target: LOG_TARGET, "[drop] SubstateLock: {substate_id}");
            for lock in locks {
                debug!(target: LOG_TARGET, "  - {lock}");
            }
        }

        for transaction in &self.new_transactions_to_sequence {
            debug!(target: LOG_TARGET, "[drop] Sequence new transaction: {}", transaction.id());
        }

        for (transaction_id, change) in &self.transaction_changes {
            debug!(target: LOG_TARGET, "[drop] TransactionChange: {transaction_id}");
            if let Some(ref execution) = change.execution {
                debug!(target: LOG_TARGET, "  - {execution}");
            }
            if let Some(ref update) = change.next_update {
                debug!(target: LOG_TARGET, "  - Update: {} {} {}", update.transaction_id(), update.decision(), update.transaction().current_stage());
            }
            for (shard_group, pledges) in &change.foreign_pledges {
                debug!(target: LOG_TARGET, "  - ForeignPledges: {shard_group}");
                for pledge in pledges {
                    debug!(target: LOG_TARGET, "    - {pledge}");
                }
            }
        }

        for block_id in &self.proposed_foreign_proposals {
            debug!(target: LOG_TARGET, "[drop] ProposedForeignProposal: {block_id}");
        }

        for mint in &self.proposed_utxo_mints {
            debug!(target: LOG_TARGET, "[drop] ProposedUtxoMint: {mint}");
        }
    }
}

impl Display for ProposedBlockChangeSet {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProposedBlockChangeSet({}, ", self.block)?;
        match self.quorum_decision {
            Some(decision) => write!(f, " Decision: {},", decision)?,
            None => write!(f, " Decision: NO VOTE, ")?,
        }
        if !self.local_substate_changes.is_empty() {
            write!(f, " BlockDiff: {} change(s), ", self.local_substate_changes.len())?;
        }
        if !self.state_tree_diffs.is_empty() {
            write!(f, " StateTreeDiff: {} change(s), ", self.state_tree_diffs.len())?;
        }
        if !self.new_transactions_to_sequence.is_empty() {
            write!(
                f,
                " NewTransactions: {} transaction(s), ",
                self.new_transactions_to_sequence.len()
            )?;
        }
        if !self.substate_locks.is_empty() {
            write!(f, " SubstateLocks: {} lock(s), ", self.substate_locks.len())?;
        }
        if !self.transaction_changes.is_empty() {
            write!(f, " TransactionChanges: {} change(s), ", self.transaction_changes.len())?;
        }
        if !self.proposed_foreign_proposals.is_empty() {
            write!(
                f,
                " ProposedForeignProposals: {} proposal(s), ",
                self.proposed_foreign_proposals.len()
            )?;
        }
        if !self.proposed_utxo_mints.is_empty() {
            write!(f, " ProposedUtxoMints: {} mint(s), ", self.proposed_utxo_mints.len())?;
        }
        write!(f, ")")
    }
}

#[derive(Debug, Clone, Default)]
struct TransactionChangeSet {
    execution: Option<BlockTransactionExecution>,
    next_update: Option<TransactionPoolStatusUpdate>,
    foreign_pledges: HashMap<ShardGroup, SubstatePledges>,
}

impl TransactionChangeSet {
    pub fn apply_update(&self, tx_rec_mut: &mut TransactionPoolRecord) {
        if let Some(update) = self.next_update.as_ref() {
            update.apply(tx_rec_mut);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use tari_consensus_types::Decision;
    use tari_engine_types::commit_result::AbortReason;
    use tari_ootle_common_types::{Epoch, NumPreshards, VersionedSubstateId};
    use tari_ootle_storage::consensus_models::{SubstatePledge, TransactionPoolStage};
    use tari_template_lib_types::ComponentAddress;

    use super::*;

    #[test]
    fn taking_executions_keeps_the_pending_transaction_update() {
        let leaf = LeafBlock {
            block_id: BlockId::zero(),
            height: NodeHeight(1),
            epoch: Epoch(1),
            shard_group: ShardGroup::all_shards(NumPreshards::P256),
        };
        let mut change_set = ProposedBlockChangeSet::new(leaf);

        let mut aborted = local_prepared_record();
        aborted.set_local_decision(Decision::Abort(AbortReason::ExecutionFailure));
        let transaction_id = *aborted.id();
        change_set.set_next_transaction_update(aborted).unwrap();
        change_set.add_foreign_pledges(&transaction_id, ShardGroup::all_shards(NumPreshards::P256), vec![
            SubstatePledge::Output {
                substate_id: VersionedSubstateId::new(SubstateId::Component(ComponentAddress::from_array([1; 32])), 0),
            },
        ]);

        assert_eq!(change_set.take_transaction_executions().len(), 0);

        let mut committing = local_prepared_record();
        change_set.apply_transaction_update(&mut committing);
        assert!(
            committing.current_decision().is_abort(),
            "pending update must survive execution harvesting so later commands in the block see it"
        );
        assert_eq!(
            change_set.get_foreign_pledges(&transaction_id).count(),
            1,
            "foreign pledges must survive execution harvesting so the transaction can still be executed"
        );
    }

    fn local_prepared_record() -> TransactionPoolRecord {
        TransactionPoolRecord::load(
            TransactionId::new([1; 32]),
            Evidence::default(),
            false,
            0,
            None,
            TransactionPoolStage::LocalPrepared,
            None,
            Decision::Commit,
            None,
            None,
            true,
            Epoch(1),
            None,
            time::OffsetDateTime::now_utc(),
            None,
            0,
            0,
        )
    }

    #[test]
    fn check_max_mem_usage() {
        let sz = size_of::<ProposedBlockChangeSet>();
        eprintln!("ProposedBlockChangeSet: {}", sz);
        let mem_block_diff = size_of::<SubstateChange>() * MEM_MAX_BLOCK_DIFF_CHANGES;
        eprintln!("mem_block_diff: {}KiB", mem_block_diff / 1024);
        let mem_state_tree_diffs =
            size_of::<Shard>() * size_of::<PendingShardStateTreeDiff>() * MEM_MAX_STATE_TREE_DIFF_SIZE;
        eprintln!("mem_state_tree_diffs: {}", mem_state_tree_diffs);
        let mem_substate_locks = (size_of::<SubstateId>() + size_of::<SubstateLock>()) * MEM_MAX_SUBSTATE_LOCK_SIZE;
        eprintln!("mem_substate_locks: {}MiB", mem_substate_locks / 1024 / 1024);
        let mem_transaction_changes =
            (size_of::<TransactionId>() + size_of::<TransactionChangeSet>()) * MEM_MAX_TRANSACTION_CHANGE_SIZE;
        eprintln!("mem_transaction_changes: {}", mem_transaction_changes);
        let mem_new_sequence_transactions = size_of::<TransactionPoolRecord>() * MEM_NEW_SEQUENCE_TRANSACTIONS;
        eprintln!("mem_new_sequence_transactions: {}", mem_new_sequence_transactions);
        let mem_proposed_foreign_proposals = size_of::<BlockId>() * MEM_MAX_PROPOSED_FOREIGN_PROPOSALS_SIZE;
        eprintln!("mem_proposed_foreign_proposals: {}", mem_proposed_foreign_proposals);
        let mem_proposed_utxo_mints = size_of::<SubstateId>() * MEM_MAX_PROPOSED_UTXO_MINTS_SIZE;
        eprintln!("mem_proposed_utxo_mints: {}", mem_proposed_utxo_mints);
        let total_mem = mem_block_diff +
            mem_state_tree_diffs +
            mem_substate_locks +
            mem_transaction_changes +
            mem_new_sequence_transactions +
            mem_proposed_foreign_proposals +
            mem_proposed_utxo_mints;
        // Last checked: 2024-06-03 (13,368000 bytes)
        assert!(
            total_mem < 14 * 1_000_000,
            "Estimated max memory usage of ProposedBlockChangeSet is too high: {} bytes",
            total_mem
        );
    }
}
