//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, fmt::Display, ops::Deref};

use indexmap::IndexMap;
use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_consensus_types::BlockId;
use tari_ootle_common_types::shard::Shard;
use tari_state_tree::{IndexedTreeDiff, StateHashTreeDiff, StateTreePayload, TransitionAccumulator, Version};

use crate::{StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

/// The diff a pending block makes to one shard's state tree, with the version and transition accumulator the shard
/// is at once it applies. Cloning shares the diff.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
pub struct PendingShardStateTreeDiff {
    #[n(0)]
    pub version: Version,
    #[n(1)]
    pub diff: IndexedTreeDiff<StateTreePayload>,
    #[n(2)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    pub transition_acc: TransitionAccumulator,
}

impl PendingShardStateTreeDiff {
    pub fn new(
        version: Version,
        transition_acc: TransitionAccumulator,
        diff: StateHashTreeDiff<StateTreePayload>,
    ) -> Self {
        Self {
            version,
            diff: IndexedTreeDiff::new(diff),
            transition_acc,
        }
    }

    /// The version and accumulator the shard is at once this diff is committed.
    pub fn committed_version(&self) -> CommittedShardVersion {
        CommittedShardVersion {
            version: self.version,
            transition_acc: self.transition_acc,
        }
    }
}

/// The state version a block moved a shard to, with the transition accumulator at that version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommittedShardVersion {
    pub version: Version,
    pub transition_acc: TransitionAccumulator,
}

impl PendingShardStateTreeDiff {
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
        tx.pending_state_tree_diffs_remove_and_return_by_block(block_id)
    }

    /// Records the diffs `block_id` makes, one per shard.
    pub fn create_all<'a, TTx, I>(tx: &mut TTx, block_id: &BlockId, diffs: I) -> Result<(), StorageError>
    where
        TTx: Deref + StateStoreWriteTransaction,
        TTx::Target: StateStoreReadTransaction,
        I: IntoIterator<Item = (&'a Shard, &'a PendingShardStateTreeDiff)>,
    {
        tx.pending_state_tree_diffs_insert_all(block_id, diffs)
    }
}

impl Display for PendingShardStateTreeDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PendingShardStateTreeDiff(v{}, acc {}, {} new node(s), {} stale node(s))",
            self.version,
            self.transition_acc,
            self.diff.new_nodes.len(),
            self.diff.stale_tree_nodes.len()
        )
    }
}
