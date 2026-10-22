//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, ops::Deref};

use indexmap::IndexMap;
use log::*;
use tari_engine_types::ProtocolVersion;
use tari_ootle_common_types::{ShardGroup, shard::Shard};
use tari_ootle_storage::{
    ShardScopedTreeStoreReader,
    ShardScopedTreeStoreWriter,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::PendingShardStateTreeDiff,
};
use tari_state_tree::{
    JmtStorageError,
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    SpreadPrefixStateTree,
    StagedTreeStore,
    StateHashTreeDiff,
    StateTreeError,
    StateTreePayload,
    SubstateTreeChange,
    TreeHash,
    Version,
    compute_shard_group_root,
};

const LOG_TARGET: &str = "tari::ootle::consensus::sharded_state_tree";

pub struct ShardedStateTree<TTx> {
    tx: TTx,
    pending_diffs: HashMap<Shard, Vec<PendingShardStateTreeDiff>>,
    shard_tree_diffs: IndexMap<Shard, PendingShardStateTreeDiff>,
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

    pub fn into_shard_tree_diffs(self) -> IndexMap<Shard, PendingShardStateTreeDiff> {
        self.shard_tree_diffs
    }

    /// Applies `changes` and returns the resulting shard group state merkle root, formed under `protocol_version`.
    pub fn put_substate_tree_changes(
        &mut self,
        protocol_version: ProtocolVersion,
        shard_group: ShardGroup,
        changes: IndexMap<Shard, Vec<SubstateTreeChange>>,
    ) -> Result<TreeHash, StateTreeError> {
        let mut shard_state_roots = HashMap::with_capacity(changes.len());
        for (shard, changes) in changes {
            let current_version = self.get_current_version(shard)?;
            let next_version = match current_version {
                Some(current_version) => current_version
                    .checked_add(1)
                    .ok_or(StateTreeError::VersionExhausted { current_version })?,
                None => 1,
            };

            // Read only state store that is scoped to the shard
            let scoped_store = ShardScopedTreeStoreReader::new(self.tx, shard);
            // Staged store that tracks changes to the state tree
            let mut store = StagedTreeStore::new(&scoped_store);
            // Apply pending (not yet committed) diffs to the staged store
            if let Some(diffs) = self.pending_diffs.remove(&shard) {
                let mut num_changes = 0usize;
                let num_diffs = diffs.len();
                let last_version = diffs.last().map(|d| d.version).unwrap_or(0);
                for diff in diffs {
                    num_changes += diff.diff.new_nodes.len() + diff.diff.stale_tree_nodes.len();
                    store.apply_pending_diff(diff.diff);
                }
                debug!(
                    target: LOG_TARGET,
                    "Applied {num_diffs} pending diff(s) ({num_changes} change(s)) to shard {shard} (version={last_version})",
                );
            }

            // Apply state updates to the state tree that is backed by the staged shard-scoped store
            let mut state_tree = SpreadPrefixStateTree::new(&mut store);
            debug!(target: LOG_TARGET, "v{next_version} contains {} new tree change(s) for shard {shard}", changes.len());
            let shard_state_hash = state_tree.put_substate_changes(current_version, next_version, changes)?;
            shard_state_roots.insert(shard, (shard_state_hash, next_version));
            self.shard_tree_diffs
                .insert(shard, PendingShardStateTreeDiff::new(next_version, store.into_diff()));
        }

        let root_hash = self.get_shard_group_root(protocol_version, shard_group, shard_state_roots)?;
        Ok(root_hash)
    }

    pub fn calculate_state_root(
        &self,
        protocol_version: ProtocolVersion,
        shard_group: ShardGroup,
    ) -> Result<TreeHash, StateTreeError> {
        self.get_shard_group_root(protocol_version, shard_group, HashMap::new())
    }

    /// `shard_state_roots` holds the root and state version of each shard this block changes. Every other shard
    /// contributes its current root and version.
    fn get_shard_group_root(
        &self,
        protocol_version: ProtocolVersion,
        shard_group: ShardGroup,
        mut shard_state_roots: HashMap<Shard, (TreeHash, Version)>,
    ) -> Result<TreeHash, StateTreeError> {
        let mut shard_states = Vec::with_capacity(shard_group.len() + 1);
        for shard in shard_group.shard_iter_with_global() {
            let (root, version) = match shard_state_roots.remove(&shard) {
                Some(state) => state,
                None => self.get_state_root_for_shard(shard)?,
            };
            shard_states.push((shard, root, version));
        }
        let hash = compute_shard_group_root(protocol_version, shard_states)?;
        Ok(hash)
    }

    /// The current root and state version of `shard`. A shard with no state changes is at version 0 with the empty
    /// tree root.
    fn get_state_root_for_shard(&self, shard: Shard) -> Result<(TreeHash, Version), StateTreeError> {
        let Some(version) = self.get_current_version(shard)? else {
            return Ok((SPARSE_MERKLE_PLACEHOLDER_HASH, 0));
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
        Ok((root_hash, version))
    }
}

impl<TTx> ShardedStateTree<&mut TTx>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
{
    pub fn commit_diffs(
        &mut self,
        diffs: IndexMap<Shard, Vec<PendingShardStateTreeDiff>>,
    ) -> Result<HashMap<Shard, Version>, StateTreeError> {
        debug!(
            target: LOG_TARGET,
            "Committing {} pending diff(s) for {} shard(s)",
            diffs.values().map(|v| v.len()).sum::<usize>(),
            diffs.len()
        );

        let mut state_versions = HashMap::with_capacity(diffs.len());
        for (shard, pending_diffs) in diffs {
            for pending_diff in pending_diffs {
                state_versions.insert(shard, pending_diff.version);
                self.commit_diff(shard, pending_diff.version, &pending_diff.diff)?;
            }
        }

        Ok(state_versions)
    }

    pub fn commit_diff(
        &mut self,
        shard: Shard,
        version: Version,
        diff: &StateHashTreeDiff<StateTreePayload>,
    ) -> Result<(), StateTreeError> {
        let mut store = ShardScopedTreeStoreWriter::new(self.tx, shard);

        trace!(
            target: LOG_TARGET,
            "Committing diff for shard {shard} (version={version}) with {} new node(s) and {} stale node(s)",
            diff.new_nodes.len(),
            diff.stale_tree_nodes.len()
        );
        store.record_stale_tree_nodes(version, &diff.stale_tree_nodes)?;
        store.insert_nodes(&diff.new_nodes)?;
        store.set_state_version(version)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tari_ootle_common_types::NumPreshards;
    use tari_ootle_storage::StateStore;
    use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};

    use super::*;

    /// A shard whose state tree has reached the last version a `Version` can hold has no next
    /// version to write changes at.
    #[test]
    fn a_shard_at_the_last_version_cannot_take_changes() {
        let dir = tempfile::tempdir().unwrap();
        let store = RocksDbStateStore::<String>::open(dir.path().join("db"), DatabaseOptions::default()).unwrap();
        let tx = store.create_read_tx().unwrap();
        let shard = Shard::first();
        let mut tree = ShardedStateTree::new(&tx).with_pending_diffs(HashMap::from([(shard, vec![
            PendingShardStateTreeDiff::new(Version::MAX, StateHashTreeDiff::new()),
        ])]));

        let result = tree.put_substate_tree_changes(
            ProtocolVersion::V0,
            ShardGroup::all_shards(NumPreshards::P256),
            IndexMap::from([(shard, vec![])]),
        );

        assert!(
            matches!(
                result,
                Err(StateTreeError::VersionExhausted {
                    current_version: Version::MAX
                })
            ),
            "{result:?}"
        );
    }
}
