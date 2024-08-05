//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::marker::PhantomData;

use serde::{Deserialize, Serialize};
use tari_engine_types::substate::SubstateId;

use crate::{
    error::StateTreeError,
    jellyfish::{Hash, JellyfishMerkleTree, SparseMerkleProofExt, TreeStore, Version},
    key_mapper::{DbKeyMapper, HashIdentityKeyMapper, SpreadPrefixKeyMapper},
    Node,
    NodeKey,
    ProofValue,
    StaleTreeNode,
    TreeStoreReader,
    TreeUpdateBatch,
};

pub type SpreadPrefixStateTree<'a, S> = StateTree<'a, S, SpreadPrefixKeyMapper>;
pub type RootStateTree<'a, S> = StateTree<'a, S, HashIdentityKeyMapper>;

pub struct StateTree<'a, S, M> {
    store: &'a mut S,
    _mapper: PhantomData<M>,
}

impl<'a, S, M> StateTree<'a, S, M> {
    pub fn new(store: &'a mut S) -> Self {
        Self {
            store,
            _mapper: PhantomData,
        }
    }
}

impl<'a, S: TreeStoreReader<Version>, M: DbKeyMapper<SubstateId>> StateTree<'a, S, M> {
    pub fn get_proof(
        &self,
        version: Version,
        key: &SubstateId,
    ) -> Result<(Option<ProofValue<Version>>, SparseMerkleProofExt), StateTreeError> {
        let smt = JellyfishMerkleTree::new(self.store);
        let key = M::map_to_leaf_key(key);
        let (maybe_value, proof) = smt.get_with_proof_ext(key.as_ref(), version)?;
        Ok((maybe_value, proof))
    }

    pub fn get_root_hash(&self, version: Version) -> Result<Hash, StateTreeError> {
        let smt = JellyfishMerkleTree::new(self.store);
        let root_hash = smt.get_root_hash(version)?;
        Ok(root_hash)
    }
}

impl<'a, S: TreeStore<Version>, M: DbKeyMapper<SubstateId>> StateTree<'a, S, M> {
    fn calculate_substate_changes<I: IntoIterator<Item = SubstateTreeChange>>(
        &mut self,
        current_version: Option<Version>,
        next_version: Version,
        changes: I,
    ) -> Result<(Hash, StateHashTreeDiff<Version>), StateTreeError> {
        let (root_hash, update_batch) =
            calculate_substate_changes::<_, M, _>(self.store, current_version, next_version, changes)?;
        Ok((root_hash, update_batch.into()))
    }

    /// Stores the substate changes in the state tree and returns the new root hash.
    pub fn put_substate_changes<I: IntoIterator<Item = SubstateTreeChange>>(
        &mut self,
        current_version: Option<Version>,
        next_version: Version,
        changes: I,
    ) -> Result<Hash, StateTreeError> {
        let (root_hash, update_batch) = self.calculate_substate_changes(current_version, next_version, changes)?;
        self.commit_diff(update_batch)?;
        Ok(root_hash)
    }

    pub fn commit_diff(&mut self, diff: StateHashTreeDiff<Version>) -> Result<(), StateTreeError> {
        for (key, node) in diff.new_nodes {
            log::debug!("Inserting node: {}", key);
            self.store.insert_node(key, node)?;
        }

        for stale_tree_node in diff.stale_tree_nodes {
            log::debug!("Recording stale tree node: {}", stale_tree_node.as_node_key());
            self.store.record_stale_tree_node(stale_tree_node)?;
        }

        Ok(())
    }
}

impl<'a, S: TreeStore<()>, M: DbKeyMapper<Hash>> StateTree<'a, S, M> {
    pub fn put_root_hash_changes<I: IntoIterator<Item = Hash>>(
        &mut self,
        current_version: Option<Version>,
        next_version: Version,
        changes: I,
    ) -> Result<Hash, StateTreeError> {
        let jmt = JellyfishMerkleTree::<_, ()>::new(self.store);

        let changes = changes
            .into_iter()
            .map(|hash| (M::map_to_leaf_key(&hash), Some((hash, ()))));

        let (root_hash, update_result) = jmt.batch_put_value_set(changes, None, current_version, next_version)?;

        for (k, node) in update_result.node_batch {
            self.store.insert_node(k, node)?;
        }

        for stale_tree_node in update_result.stale_node_index_batch {
            self.store
                .record_stale_tree_node(StaleTreeNode::Node(stale_tree_node.node_key))?;
        }

        Ok(root_hash)
    }
}

/// Calculates the new root hash and tree updates for the given substate changes.
fn calculate_substate_changes<
    S: TreeStoreReader<Version>,
    M: DbKeyMapper<SubstateId>,
    I: IntoIterator<Item = SubstateTreeChange>,
>(
    store: &mut S,
    current_version: Option<Version>,
    next_version: Version,
    changes: I,
) -> Result<(Hash, TreeUpdateBatch<Version>), StateTreeError> {
    let jmt = JellyfishMerkleTree::new(store);

    let changes = changes.into_iter().map(|ch| match ch {
        SubstateTreeChange::Up { id, value_hash } => (M::map_to_leaf_key(&id), Some((value_hash, next_version))),
        SubstateTreeChange::Down { id } => (M::map_to_leaf_key(&id), None),
    });

    let (root_hash, update_result) = jmt.batch_put_value_set(changes, None, current_version, next_version)?;

    Ok((root_hash, update_result))
}

pub enum SubstateTreeChange {
    Up { id: SubstateId, value_hash: Hash },
    Down { id: SubstateId },
}

impl SubstateTreeChange {
    pub fn id(&self) -> &SubstateId {
        match self {
            Self::Up { id, .. } => id,
            Self::Down { id } => id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateHashTreeDiff<P> {
    pub new_nodes: Vec<(NodeKey, Node<P>)>,
    pub stale_tree_nodes: Vec<StaleTreeNode>,
}

impl<P> StateHashTreeDiff<P> {
    pub fn new() -> Self {
        Self {
            new_nodes: Vec::new(),
            stale_tree_nodes: Vec::new(),
        }
    }
}

impl<P> From<TreeUpdateBatch<P>> for StateHashTreeDiff<P> {
    fn from(batch: TreeUpdateBatch<P>) -> Self {
        Self {
            new_nodes: batch.node_batch,
            stale_tree_nodes: batch
                .stale_node_index_batch
                .into_iter()
                .map(|node| StaleTreeNode::Node(node.node_key))
                .collect(),
        }
    }
}
