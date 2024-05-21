//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::Deref;

use indexmap::IndexMap;
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockDiff,
        LeafBlock,
        LockedSubstate,
        PendingStateTreeDiff,
        QuorumDecision,
        SubstateChange,
        SubstateRecord,
        TransactionAtom,
        TransactionExecution,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionPoolStatusUpdate,
    },
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};
use tari_engine_types::substate::SubstateId;
use tari_state_tree::StateHashTreeDiff;
use tari_transaction::TransactionId;

#[derive(Debug, Clone)]
pub struct BlockDecision {
    pub quorum_decision: Option<QuorumDecision>,
    pub locked_blocks: Vec<Block>,
    pub finalized_transactions: Vec<Vec<TransactionAtom>>,
}

#[derive(Debug, Clone)]
pub struct ProposedBlockChangeSet {
    block: LeafBlock,
    quorum_decision: Option<QuorumDecision>,
    block_diff: Vec<SubstateChange>,
    state_tree_diff: StateHashTreeDiff,
    substate_locks: IndexMap<SubstateId, Vec<LockedSubstate>>,
    transaction_changes: IndexMap<TransactionId, TransactionChangeSet>,
}

impl ProposedBlockChangeSet {
    pub fn new(block: LeafBlock) -> Self {
        Self {
            block,
            quorum_decision: None,
            block_diff: Vec::new(),
            substate_locks: IndexMap::new(),
            transaction_changes: IndexMap::new(),
            state_tree_diff: StateHashTreeDiff::default(),
        }
    }

    pub fn no_vote(mut self) -> Self {
        self.quorum_decision = None;
        self.block_diff = Vec::new();
        self.transaction_changes.clear();
        self
    }

    pub fn set_state_tree_diff(&mut self, diff: StateHashTreeDiff) -> &mut Self {
        self.state_tree_diff = diff;
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

    pub fn set_substate_locks(&mut self, locks: IndexMap<SubstateId, Vec<LockedSubstate>>) -> &mut Self {
        self.substate_locks = locks;
        self
    }

    pub fn is_accept(&self) -> bool {
        matches!(self.quorum_decision, Some(QuorumDecision::Accept))
    }

    pub fn quorum_decision(&self) -> Option<QuorumDecision> {
        self.quorum_decision
    }

    pub fn add_transaction_execution(&mut self, execution: TransactionExecution) -> &mut Self {
        let change_mut = self.transaction_changes.entry(*execution.transaction_id()).or_default();
        change_mut.execution = Some(execution);
        self
    }

    pub fn get_transaction_execution(&self, transaction_id: &TransactionId) -> Option<&TransactionExecution> {
        self.transaction_changes
            .get(transaction_id)
            .and_then(|change| change.execution.as_ref())
    }

    pub fn set_next_transaction_update(
        &mut self,
        transaction: &TransactionPoolRecord,
        next_stage: TransactionPoolStage,
        is_ready: bool,
    ) -> &mut Self {
        let change_mut = self
            .transaction_changes
            .entry(*transaction.transaction_id())
            .or_default();
        change_mut.next_update = Some(TransactionPoolStatusUpdate {
            block_id: self.block.block_id,
            block_height: self.block.height,
            transaction_id: *transaction.transaction_id(),
            stage: next_stage,
            evidence: transaction.atom().evidence.clone(),
            is_ready,
            local_decision: transaction.current_decision(),
        });
        self
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

        // Store the tree diff
        PendingStateTreeDiff::new(*self.block.block_id(), self.block.height(), self.state_tree_diff).save(tx)?;

        // Save locks
        SubstateRecord::insert_all_locks(tx, self.block.block_id, self.substate_locks)?;

        for change in self.transaction_changes.values() {
            // Save any transaction executions for the block
            if let Some(ref execution) = change.execution {
                // This may already exist if we proposed the block
                execution.insert_if_required(tx)?;
            }

            // Save any transaction pool updates
            if let Some(ref update) = change.next_update {
                update.insert(tx)?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct TransactionChangeSet {
    execution: Option<TransactionExecution>,
    next_update: Option<TransactionPoolStatusUpdate>,
}
