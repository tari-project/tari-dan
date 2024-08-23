//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::Deref;

use indexmap::IndexMap;
use log::*;
use tari_dan_common_types::{shard::Shard, Epoch};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockDiff,
        BlockId,
        BlockTransactionExecution,
        ForeignProposal,
        LeafBlock,
        PendingShardStateTreeDiff,
        QuorumCertificate,
        QuorumDecision,
        SubstateChange,
        SubstateLock,
        SubstateRecord,
        TransactionExecution,
        TransactionPoolError,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionPoolStatusUpdate,
        VersionedStateHashTreeDiff,
    },
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};
use tari_engine_types::substate::SubstateId;
use tari_transaction::TransactionId;

const LOG_TARGET: &str = "tari::dan::consensus::block_change_set";

#[derive(Debug, Clone)]
pub struct BlockDecision {
    pub quorum_decision: Option<QuorumDecision>,
    /// Contains newly-locked non-dummy blocks and the QC that justifies each block i.e. typically the parent block's
    /// QC
    pub locked_blocks: Vec<(Block, QuorumCertificate)>,
    pub finalized_transactions: Vec<Vec<TransactionPoolRecord>>,
    pub end_of_epoch: Option<Epoch>,
}

#[derive(Debug, Clone)]
pub struct ProposedBlockChangeSet {
    block: LeafBlock,
    quorum_decision: Option<QuorumDecision>,
    block_diff: Vec<SubstateChange>,
    state_tree_diffs: IndexMap<Shard, VersionedStateHashTreeDiff>,
    substate_locks: IndexMap<SubstateId, Vec<SubstateLock>>,
    transaction_changes: IndexMap<TransactionId, TransactionChangeSet>,
    proposed_foreign_proposals: Vec<BlockId>,
}

impl ProposedBlockChangeSet {
    pub fn new(block: LeafBlock) -> Self {
        Self {
            block,
            quorum_decision: None,
            block_diff: Vec::new(),
            substate_locks: IndexMap::new(),
            transaction_changes: IndexMap::new(),
            state_tree_diffs: IndexMap::new(),
            proposed_foreign_proposals: Vec::new(),
        }
    }

    pub fn no_vote(mut self) -> Self {
        self.quorum_decision = None;
        self.block_diff = Vec::new();
        self.transaction_changes = IndexMap::new();
        self.state_tree_diffs = IndexMap::new();
        self.substate_locks = IndexMap::new();
        self.proposed_foreign_proposals = Vec::new();
        self
    }

    pub fn set_state_tree_diffs(&mut self, diffs: IndexMap<Shard, VersionedStateHashTreeDiff>) -> &mut Self {
        self.state_tree_diffs = diffs;
        self
    }

    pub fn set_quorum_decision(&mut self, decision: QuorumDecision) -> &mut Self {
        self.quorum_decision = Some(decision);
        self
    }

    pub fn set_block_diff(&mut self, diff: Vec<SubstateChange>) -> &mut Self {
        self.block_diff = diff;
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

    // TODO: this is a hack to allow the update to be modified after the fact. This should be removed.
    pub fn next_update_mut(&mut self, transaction_id: &TransactionId) -> Option<&mut TransactionPoolStatusUpdate> {
        self.transaction_changes
            .get_mut(transaction_id)
            .and_then(|change| change.next_update.as_mut())
    }

    pub fn is_accept(&self) -> bool {
        matches!(self.quorum_decision, Some(QuorumDecision::Accept))
    }

    pub fn quorum_decision(&self) -> Option<QuorumDecision> {
        self.quorum_decision
    }

    pub fn add_transaction_execution(
        &mut self,
        execution: TransactionExecution,
    ) -> Result<&mut Self, TransactionPoolError> {
        let execution = execution.for_block(self.block.block_id);
        let change_mut = self.transaction_changes.entry(*execution.transaction_id()).or_default();
        if change_mut.execution.is_some() {
            return Err(TransactionPoolError::TransactionAlreadyExecuted {
                transaction_id: *execution.transaction_id(),
                block_id: self.block.block_id,
            });
        }

        change_mut.execution = Some(execution);
        Ok(self)
    }

    pub fn set_next_transaction_update(
        &mut self,
        transaction: &TransactionPoolRecord,
        next_stage: TransactionPoolStage,
        is_ready: bool,
    ) -> Result<&mut Self, TransactionPoolError> {
        transaction.check_pending_status_update(next_stage, is_ready)?;

        let change_mut = self
            .transaction_changes
            .entry(*transaction.transaction_id())
            .or_default();
        if change_mut.next_update.is_some() {
            return Err(TransactionPoolError::TransactionAlreadyUpdated {
                transaction_id: *transaction.transaction_id(),
                block_id: self.block.block_id,
            });
        }
        info!(
            target: LOG_TARGET,
            "📝 Setting next update for transaction {} to {:?},{},is_ready={} in block {}",
            transaction.transaction_id(),
            next_stage,
            transaction.current_decision(),
            is_ready,
            self.block.block_id
        );

        change_mut.next_update = Some(TransactionPoolStatusUpdate {
            block_id: self.block.block_id,
            transaction_id: *transaction.transaction_id(),
            stage: next_stage,
            evidence: transaction.evidence().clone(),
            is_ready,
            local_decision: transaction.current_decision(),
        });
        Ok(self)
    }
}

impl ProposedBlockChangeSet {
    pub fn save<TTx>(self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + Deref,
        TTx::Target: StateStoreReadTransaction,
    {
        let block_diff = BlockDiff::new(self.block.block_id, self.block_diff);
        // Store the block diff
        block_diff.insert(tx)?;

        // Store the tree diffs for each effected shard
        let shard_tree_diffs = self.state_tree_diffs;
        for (shard, diff) in shard_tree_diffs {
            PendingShardStateTreeDiff::create(tx, *self.block.block_id(), shard, diff)?;
        }

        // Save locks
        SubstateRecord::lock_all(tx, self.block.block_id, self.substate_locks)?;

        for change in self.transaction_changes.values() {
            // Save any transaction executions for the block
            if let Some(ref execution) = change.execution {
                // This may already exist if we proposed the block
                if execution.insert_if_required(tx)? {
                    info!(
                        target: LOG_TARGET,
                        "📝 Transaction execution for {} saved in block {}",
                        execution.transaction_id(),
                        self.block.block_id
                    );
                } else {
                    info!(
                        target: LOG_TARGET,
                        "📝 Transaction execution for {} already exists in block {}",
                        execution.transaction_id(),
                        self.block.block_id
                    );
                }
            }

            // Save any transaction pool updates
            if let Some(ref update) = change.next_update {
                update.insert(tx)?;
            }
        }

        for block_id in self.proposed_foreign_proposals {
            ForeignProposal::set_proposed_in(tx, &block_id, &self.block.block_id)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct TransactionChangeSet {
    execution: Option<BlockTransactionExecution>,
    next_update: Option<TransactionPoolStatusUpdate>,
}
