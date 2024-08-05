//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, ops::Deref};

use indexmap::IndexMap;
use tari_dan_common_types::shard::Shard;
use tari_state_tree::{StateHashTreeDiff, Version};

use crate::{consensus_models::BlockId, StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

#[derive(Debug, Clone, Default)]
pub struct PendingStateTreeDiff {
    pub version: Version,
    pub diff: StateHashTreeDiff,
}

impl PendingStateTreeDiff {
    pub fn load(version: Version, diff: StateHashTreeDiff) -> Self {
        Self { version, diff }
    }
}

impl PendingStateTreeDiff {
    /// Returns all pending state tree diffs from the last committed block (exclusive) to the given block (inclusive).
    pub fn get_all_up_to_commit_block<TTx>(
        tx: &TTx,
        block_id: &BlockId,
    ) -> Result<HashMap<Shard, Vec<Self>>, StorageError>
    where
        TTx: StateStoreReadTransaction,
    {
        tx.pending_state_tree_diffs_get_all_up_to_commit_block(block_id)
    }

    pub fn remove_by_block<TTx>(tx: &mut TTx, block_id: &BlockId) -> Result<IndexMap<Shard, Vec<Self>>, StorageError>
    where
        TTx: Deref + StateStoreWriteTransaction,
        TTx::Target: StateStoreReadTransaction,
    {
        tx.pending_state_tree_diffs_remove_by_block(block_id)
    }

    pub fn create<TTx>(
        tx: &mut TTx,
        block_id: BlockId,
        shard: Shard,
        diff: VersionedStateHashTreeDiff,
    ) -> Result<(), StorageError>
    where
        TTx: Deref + StateStoreWriteTransaction,
        TTx::Target: StateStoreReadTransaction,
    {
        // if tx.pending_state_tree_diffs_exists_for_block(&block_id)? {
        //     Ok(false)
        // } else {
        tx.pending_state_tree_diffs_insert(block_id, shard, diff)?;
        // Ok(true)
        // }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct VersionedStateHashTreeDiff {
    pub version: Version,
    pub diff: StateHashTreeDiff,
}

impl VersionedStateHashTreeDiff {
    pub fn new(version: Version, diff: StateHashTreeDiff) -> Self {
        Self { version, diff }
    }
}
