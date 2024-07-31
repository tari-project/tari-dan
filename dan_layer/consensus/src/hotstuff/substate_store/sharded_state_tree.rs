//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use indexmap::IndexMap;
use log::debug;
use tari_dan_common_types::{hashing::state_root_hasher, shard::Shard};
use tari_dan_storage::{
    consensus_models::{PendingStateTreeDiff, VersionedStateHashTreeDiff},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
};
use tari_state_tree::{
    Hash,
    JmtStorageError,
    SpreadPrefixStateTree,
    StagedTreeStore,
    StateHashTreeDiff,
    StateTreeError,
    SubstateTreeChange,
    TreeStoreWriter,
    Version,
};

use crate::hotstuff::substate_store::sharded_store::{ShardScopedTreeStoreReader, ShardScopedTreeStoreWriter};

const LOG_TARGET: &str = "tari::dan::consensus::sharded_state_tree";

pub struct ShardedStateTree<TTx> {
    tx: TTx,
    pending_diffs: HashMap<Shard, Vec<PendingStateTreeDiff>>,
    sharded_tree_diffs: IndexMap<Shard, VersionedStateHashTreeDiff>,
}

impl<TTx> ShardedStateTree<TTx> {
    pub fn new(tx: TTx) -> Self {
        Self {
            tx,
            pending_diffs: HashMap::new(),
            sharded_tree_diffs: IndexMap::new(),
        }
    }

    pub fn with_pending_diffs(self, pending_diffs: HashMap<Shard, Vec<PendingStateTreeDiff>>) -> Self {
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
            .map_err(|e| StateTreeError::StorageError(JmtStorageError::UnexpectedError(e.to_string())))?;
        Ok(maybe_version)
    }

    pub fn into_versioned_tree_diffs(self) -> IndexMap<Shard, VersionedStateHashTreeDiff> {
        self.sharded_tree_diffs
    }

    pub fn put_substate_tree_changes(
        &mut self,
        changes: IndexMap<Shard, Vec<SubstateTreeChange>>,
    ) -> Result<Hash, StateTreeError> {
        // This is here so that the state merkle root is all zeros for no changes (instead of being
        // state_root_hasher().result()).
        if changes.is_empty() {
            return Ok(Hash::zero());
        }

        let mut state_roots = state_root_hasher();
        for (shard, changes) in changes {
            let current_version = self.get_current_version(shard)?;
            let next_version = current_version.unwrap_or(0) + 1;

            // Read only state store that is scoped to the shard
            let scoped_store = ShardScopedTreeStoreReader::new(self.tx, shard);
            // Staged store that tracks changes to the state tree
            let mut store = StagedTreeStore::new(&scoped_store);
            // Apply pending (not yet committed) diffs to the staged store
            if let Some(diffs) = self.pending_diffs.remove(&shard) {
                debug!(target: LOG_TARGET, "Applying {num_diffs} pending diff(s) to shard {shard} (version={version})", num_diffs = diffs.len(), version = diffs.last().map(|d| d.version).unwrap_or(0));
                for diff in diffs {
                    store.apply_pending_diff(diff.diff);
                }
            }

            // Apply state updates to the state tree that is backed by the staged shard-scoped store
            let mut state_tree = SpreadPrefixStateTree::new(&mut store);
            debug!(target: LOG_TARGET, "v{next_version} contains {} tree change(s) for shard {shard}", changes.len());
            let state_root = state_tree.put_substate_changes(current_version, next_version, changes)?;
            state_roots.update(&state_root);
            self.sharded_tree_diffs
                .insert(shard, VersionedStateHashTreeDiff::new(next_version, store.into_diff()));
        }

        // TODO: use a Merkle tree to generate a root for these hashes
        Ok(state_roots.result())
    }
}

impl<TTx: StateStoreWriteTransaction> ShardedStateTree<&mut TTx> {
    pub fn commit_diffs(&mut self, diffs: IndexMap<Shard, Vec<PendingStateTreeDiff>>) -> Result<(), StateTreeError> {
        for (shard, pending_diffs) in diffs {
            for pending_diff in pending_diffs {
                let version = pending_diff.version;
                let diff = pending_diff.diff;
                self.commit_diff(shard, version, diff)?;
            }
        }

        Ok(())
    }

    pub fn commit_diff(
        &mut self,
        shard: Shard,
        version: Version,
        diff: StateHashTreeDiff,
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
