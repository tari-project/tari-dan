//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use tari_dan_common_types::NodeHeight;

use crate::{consensus_models::BlockId, StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

#[derive(Debug, Clone)]
pub struct PendingStateTreeDiff {
    pub block_id: BlockId,
    pub block_height: NodeHeight,
    pub diff: tari_state_tree::StateHashTreeDiff,
}

impl PendingStateTreeDiff {
    pub fn new(block_id: BlockId, block_height: NodeHeight, diff: tari_state_tree::StateHashTreeDiff) -> Self {
        Self {
            block_id,
            block_height,
            diff,
        }
    }
}

impl PendingStateTreeDiff {
    /// Returns all pending state tree diffs from the last committed block (exclusive) to the given block (inclusive).
    pub fn get_all_up_to_commit_block<TTx>(tx: &mut TTx, block_id: &BlockId) -> Result<Vec<Self>, StorageError>
    where TTx: StateStoreReadTransaction {
        tx.pending_state_tree_diffs_get_all_up_to_commit_block(block_id)
    }

    pub fn remove_by_block<TTx>(tx: &mut TTx, block_id: &BlockId) -> Result<Self, StorageError>
    where
        TTx: DerefMut + StateStoreWriteTransaction,
        TTx::Target: StateStoreReadTransaction,
    {
        tx.pending_state_tree_diffs_remove_by_block(block_id)
    }

    pub fn save<TTx>(&self, tx: &mut TTx) -> Result<bool, StorageError>
    where
        TTx: DerefMut + StateStoreWriteTransaction,
        TTx::Target: StateStoreReadTransaction,
    {
        if tx
            .deref_mut()
            .pending_state_tree_diffs_exists_for_block(&self.block_id)?
        {
            Ok(false)
        } else {
            tx.pending_state_tree_diffs_insert(self)?;
            Ok(true)
        }
    }
}
