//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::marker::PhantomData;

use serde::{Deserialize, Serialize};
use tari_engine_types::substate::SubstateId;

use crate::{
    error::StateTreeError,
    jellyfish::{Hash, JellyfishMerkleTree, LeafKey, SparseMerkleProofExt, TreeStore, Version},
    key_mapper::{DbKeyMapper, SpreadPrefixKeyMapper},
    Node,
    NodeKey,
    ProofValue,
    StaleTreeNode,
    TreeStoreReader,
    TreeUpdateBatch,
};

pub type SpreadPrefixStateTree<'a, S> = StateTree<'a, S, SpreadPrefixKeyMapper>;

pub struct StateTree<'a, S, M> {
    store: &'a mut S,
    _mapper: PhantomData<M>,
}

struct LeafChange {
    key: LeafKey,
    new_payload: Option<(Hash, Version)>,
}

impl<'a, S, M> StateTree<'a, S, M> {
    pub fn new(store: &'a mut S) -> Self {
        Self {
            store,
            _mapper: PhantomData,
        }
    }
}

impl<'a, S: TreeStoreReader<Version>, M: DbKeyMapper> StateTree<'a, S, M> {
    pub fn get_proof(
        &self,
        version: Version,
        substate_id: &SubstateId,
    ) -> Result<(Option<ProofValue<Version>>, SparseMerkleProofExt), StateTreeError> {
        let smt = JellyfishMerkleTree::new(self.store);
        let key = M::map_to_leaf_key(substate_id);
        let (maybe_value, proof) = smt.get_with_proof_ext(key.as_ref(), version)?;
        Ok((maybe_value, proof))
    }
}

impl<'a, S: TreeStore<Version>, M: DbKeyMapper> StateTree<'a, S, M> {
    /// Stores the substate changes in the state tree and returns the new root hash.
    pub fn put_substate_changes<I: IntoIterator<Item = SubstateTreeChange>>(
        &mut self,
        current_version: Version,
        next_version: Version,
        changes: I,
    ) -> Result<Hash, StateTreeError> {
        let (root_hash, update_batch) = calculate_substate_changes::<_, M, _>(
            self.store,
            Some(current_version).filter(|v| *v > 0),
            next_version,
            changes,
        )?;

        self.commit_diff(update_batch.into())?;
        Ok(root_hash)
    }

    pub fn commit_diff(&mut self, diff: StateHashTreeDiff) -> Result<(), StateTreeError> {
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

/// Calculates the new root hash and tree updates for the given substate changes.
fn calculate_substate_changes<
    S: TreeStoreReader<Version>,
    M: DbKeyMapper,
    I: IntoIterator<Item = SubstateTreeChange>,
>(
    store: &mut S,
    current_version: Option<Version>,
    next_version: Version,
    changes: I,
) -> Result<(Hash, TreeUpdateBatch<Version>), StateTreeError> {
    let smt = JellyfishMerkleTree::new(store);

    let changes = changes
        .into_iter()
        .map(|ch| match ch {
            SubstateTreeChange::Up { id, value_hash } => LeafChange {
                key: M::map_to_leaf_key(&id),
                new_payload: Some((value_hash, next_version)),
            },
            SubstateTreeChange::Down { id } => LeafChange {
                key: M::map_to_leaf_key(&id),
                new_payload: None,
            },
        })
        .collect::<Vec<_>>();

    let (root_hash, update_result) = smt.batch_put_value_set(
        changes
            .iter()
            .map(|change| (&change.key, change.new_payload.as_ref()))
            .collect(),
        None,
        current_version,
        next_version,
    )?;

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
pub struct StateHashTreeDiff {
    pub new_nodes: Vec<(NodeKey, Node<Version>)>,
    pub stale_tree_nodes: Vec<StaleTreeNode>,
}

impl StateHashTreeDiff {
    pub fn new() -> Self {
        Self {
            new_nodes: Vec::new(),
            stale_tree_nodes: Vec::new(),
        }
    }
}

impl From<TreeUpdateBatch<Version>> for StateHashTreeDiff {
    fn from(batch: TreeUpdateBatch<Version>) -> Self {
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
