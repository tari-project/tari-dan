//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use indexmap::IndexMap;
use log::debug;
use tari_dan_common_types::{shard::Shard, ShardGroup};
use tari_dan_storage::{
    consensus_models::{PendingShardStateTreeDiff, VersionedStateHashTreeDiff},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
};
use tari_state_tree::{
    memory_store::MemoryTreeStore,
    Hash,
    JmtStorageError,
    RootStateTree,
    SpreadPrefixStateTree,
    StagedTreeStore,
    StateHashTreeDiff,
    StateTreeError,
    SubstateTreeChange,
    TreeStoreWriter,
    Version,
    SPARSE_MERKLE_PLACEHOLDER_HASH,
};

use crate::hotstuff::substate_store::shard_state_store::{ShardScopedTreeStoreReader, ShardScopedTreeStoreWriter};

const LOG_TARGET: &str = "tari::dan::consensus::sharded_state_tree";

pub struct ShardedStateTree<TTx> {
    tx: TTx,
    pending_diffs: HashMap<Shard, Vec<PendingShardStateTreeDiff>>,
    shard_tree_diffs: IndexMap<Shard, VersionedStateHashTreeDiff>,
}

impl<TTx> ShardedStateTree<TTx> {
    pub fn new(tx: TTx) -> Self {
        Self {
            tx,
            pending_diffs: HashMap::new(),
            shard_tree_diffs: IndexMap::new(),
        }
    }

    pub fn with_pending_diffs(self, pending_diffs: HashMap<Shard, Vec<PendingShardStateTreeDiff>>) -> Self {
        Self { pending_diffs, ..self }
    }

    pub fn transaction(&self) -> &TTx {
        &self.tx
    }

    pub fn into_transaction(self) -> TTx {
        self.tx
    }
}

impl<TTx: StateStoreReadTransaction> ShardedStateTree<&TTx> {
    fn get_current_version(&self, shard: Shard) -> Result<Option<Version>, StateTreeError> {
        if let Some(version) = self
            .pending_diffs
            .get(&shard)
            .and_then(|diffs| diffs.last())
            .map(|diff| diff.version)
        {
            return Ok(Some(version));
        }

        let maybe_version = self
            .tx
            .state_tree_versions_get_latest(shard)
            .map_err(|e| StateTreeError::JmtStorageError(JmtStorageError::UnexpectedError(e.to_string())))?;
        Ok(maybe_version)
    }

    pub fn into_shard_tree_diffs(self) -> IndexMap<Shard, VersionedStateHashTreeDiff> {
        self.shard_tree_diffs
    }

    pub fn put_substate_tree_changes(
        &mut self,
        shard_group: ShardGroup,
        changes: IndexMap<Shard, Vec<SubstateTreeChange>>,
    ) -> Result<Hash, StateTreeError> {
        let mut shard_state_roots = HashMap::with_capacity(changes.len());
        for (shard, changes) in changes {
            let current_version = self.get_current_version(shard)?;
            let next_version = current_version.unwrap_or(0) + 1;

            // Read only state store that is scoped to the shard
            let scoped_store = ShardScopedTreeStoreReader::new(self.tx, shard);
            // Staged store that tracks changes to the state tree
            let mut store = StagedTreeStore::new(&scoped_store);
            // Apply pending (not yet committed) diffs to the staged store
            if let Some(diffs) = self.pending_diffs.get(&shard) {
                let mut num_changes = 0usize;
                for diff in diffs {
                    num_changes += diff.diff.new_nodes.len() + diff.diff.stale_tree_nodes.len();
                    store.apply_pending_diff(diff.diff.clone());
                }
                debug!(
                    target: LOG_TARGET,
                    "Applied {num_diffs} pending diff(s) ({num_changes} change(s)) to shard {shard} (version={version})",
                    num_diffs = diffs.len(),
                    version = diffs.last().map(|d| d.version).unwrap_or(0)
                );
            }

            // Apply state updates to the state tree that is backed by the staged shard-scoped store
            let mut state_tree = SpreadPrefixStateTree::new(&mut store);
            debug!(target: LOG_TARGET, "v{next_version} contains {} tree change(s) for shard {shard}", changes.len());
            let shard_state_hash = state_tree.put_substate_changes(current_version, next_version, changes)?;
            shard_state_roots.insert(shard, shard_state_hash);
            self.shard_tree_diffs
                .insert(shard, VersionedStateHashTreeDiff::new(next_version, store.into_diff()));
        }

        let root_hash = self.get_shard_group_root(shard_group, shard_state_roots)?;
        Ok(root_hash)
    }

    fn get_shard_group_root(
        &self,
        shard_group: ShardGroup,
        mut shard_state_roots: HashMap<Shard, Hash>,
    ) -> Result<Hash, StateTreeError> {
        let mut mem_store = MemoryTreeStore::new();
        let mut root_tree = RootStateTree::new(&mut mem_store);
        let mut hashes = Vec::with_capacity(shard_group.len());
        for shard in shard_group.shard_iter() {
            match shard_state_roots.remove(&shard) {
                Some(r) => hashes.push(r),
                None => {
                    let hash = self.get_state_root_for_shard(shard)?;
                    hashes.push(hash);
                },
            };
        }
        root_tree.put_root_hash_changes(None, 1, hashes)
    }

    fn get_state_root_for_shard(&self, shard: Shard) -> Result<Hash, StateTreeError> {
        let Some(version) = self.get_current_version(shard)? else {
            // At v0 there have been no state changes
            return Ok(SPARSE_MERKLE_PLACEHOLDER_HASH);
        };

        let scoped_store = ShardScopedTreeStoreReader::new(self.tx, shard);
        let mut store = StagedTreeStore::new(&scoped_store);
        if let Some(diffs) = self.pending_diffs.get(&shard) {
            for diff in diffs {
                store.apply_pending_diff(diff.diff.clone());
            }
        }
        let state_tree = SpreadPrefixStateTree::new(&mut store);

        let root_hash = state_tree.get_root_hash(version)?;
        Ok(root_hash)
    }
}

impl<TTx: StateStoreWriteTransaction> ShardedStateTree<&mut TTx> {
    pub fn commit_diffs(
        &mut self,
        diffs: IndexMap<Shard, Vec<PendingShardStateTreeDiff>>,
    ) -> Result<(), StateTreeError> {
        for (shard, pending_diffs) in diffs {
            for pending_diff in pending_diffs {
                self.commit_diff(shard, pending_diff.version, pending_diff.diff)?;
            }
        }

        Ok(())
    }

    pub fn commit_diff(
        &mut self,
        shard: Shard,
        version: Version,
        diff: StateHashTreeDiff<Version>,
    ) -> Result<(), StateTreeError> {
        let mut store = ShardScopedTreeStoreWriter::new(self.tx, shard);

        for stale_tree_node in diff.stale_tree_nodes {
            debug!(
                "(shard={shard}) Recording stale tree node: {}",
                stale_tree_node.as_node_key()
            );
            store.record_stale_tree_node(stale_tree_node)?;
        }

        for (key, node) in diff.new_nodes {
            debug!("(shard={shard}) Inserting node: {}", key);
            store.insert_node(key, node)?;
        }

        store.set_version(version)?;
        Ok(())
    }
}
