//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Debug;

use tari_dan_common_types::{committee::CommitteeInfo, optional::Optional};
use tari_engine_types::substate::SubstateId;

use crate::{
    consensus_models::{substate_change::SubstateChange, BlockId},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone)]
pub struct BlockDiff {
    pub block_id: BlockId,
    pub changes: Vec<SubstateChange>,
}

impl BlockDiff {
    pub fn new(block_id: BlockId, changes: Vec<SubstateChange>) -> Self {
        Self { block_id, changes }
    }

    pub fn empty(block_id: BlockId) -> Self {
        Self::new(block_id, vec![])
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn into_filtered(self, info: &CommitteeInfo) -> Self {
        Self {
            block_id: self.block_id,
            changes: self
                .changes
                .into_iter()
                // Commit all substates included in this shard. Every involved validator commits the transaction receipt.
                .filter(|change| change.versioned_substate_id().substate_id.is_transaction_receipt() || info.includes_substate_address(&change.to_substate_address()))
                .collect(),
        }
    }

    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }

    pub fn changes(&self) -> &[SubstateChange] {
        &self.changes
    }

    pub fn into_changes(self) -> Vec<SubstateChange> {
        self.changes
    }
}

impl BlockDiff {
    pub fn insert_record<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        block_id: &BlockId,
        changes: &[SubstateChange],
    ) -> Result<(), StorageError> {
        tx.block_diffs_insert(block_id, changes)
    }

    pub fn remove<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        if Self::remove_any(tx, Some(self.block_id))? == 0 {
            return Err(StorageError::NotFound {
                item: "BlockDiff (remove)",
                key: self.block_id.to_string(),
            });
        }
        Ok(())
    }

    pub fn remove_any<TTx, I>(tx: &mut TTx, block_ids: I) -> Result<usize, StorageError>
    where
        TTx: StateStoreWriteTransaction,
        I: IntoIterator<Item = BlockId>,
    {
        // TODO(perf): bulk remove
        let mut removed = 0usize;
        for block_id in block_ids {
            if tx.block_diffs_remove(&block_id).optional()?.is_some() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn get_for_substate<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        block_id: &BlockId,
        substate_id: &SubstateId,
    ) -> Result<SubstateChange, StorageError> {
        tx.block_diffs_get_last_change_for_substate(block_id, substate_id)
    }
}
