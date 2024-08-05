//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, VecDeque};

use log::debug;

use crate::{JmtStorageError, Node, NodeKey, StaleTreeNode, StateHashTreeDiff, TreeStoreReader, TreeStoreWriter};

const LOG_TARGET: &str = "tari::dan::consensus::sharded_state_tree";

pub struct StagedTreeStore<'s, S, P> {
    readable_store: &'s S,
    preceding_pending_state: HashMap<NodeKey, Node<P>>,
    new_tree_nodes: HashMap<NodeKey, Node<P>>,
    new_stale_nodes: Vec<StaleTreeNode>,
}

impl<'s, S: TreeStoreReader<P>, P> StagedTreeStore<'s, S, P> {
    pub fn new(readable_store: &'s S) -> Self {
        Self {
            readable_store,
            preceding_pending_state: HashMap::new(),
            new_tree_nodes: HashMap::new(),
            new_stale_nodes: Vec::new(),
        }
    }

    pub fn apply_pending_diff(&mut self, diff: StateHashTreeDiff<P>) {
        self.preceding_pending_state.reserve(diff.new_nodes.len());
        for (key, node) in diff.new_nodes {
            debug!(target: LOG_TARGET, "PENDING INSERT: node {}", key);
            self.preceding_pending_state.insert(key, node);
        }

        for stale in diff.stale_tree_nodes {
            debug!(target: LOG_TARGET, "PENDING DELETE: node {}", stale.as_node_key());
            if self.preceding_pending_state.remove(stale.as_node_key()).is_some() {
                debug!(target: LOG_TARGET, "PENDING DELETE: node {} removed", stale.as_node_key());
            }
        }
    }

    pub fn into_diff(self) -> StateHashTreeDiff<P> {
        StateHashTreeDiff {
            new_nodes: self.new_tree_nodes.into_iter().collect(),
            stale_tree_nodes: self.new_stale_nodes,
        }
    }
}

impl<'s, S: TreeStoreReader<P>, P: Clone> TreeStoreReader<P> for StagedTreeStore<'s, S, P> {
    fn get_node(&self, key: &NodeKey) -> Result<Node<P>, JmtStorageError> {
        if let Some(node) = self.new_tree_nodes.get(key).cloned() {
            return Ok(node);
        }
        if let Some(node) = self.preceding_pending_state.get(key).cloned() {
            return Ok(node);
        }

        self.readable_store.get_node(key)
    }
}

impl<'s, S, P> TreeStoreWriter<P> for StagedTreeStore<'s, S, P> {
    fn insert_node(&mut self, key: NodeKey, node: Node<P>) -> Result<(), JmtStorageError> {
        if self.new_tree_nodes.insert(key.clone(), node).is_some() {
            return Err(JmtStorageError::Conflict(key));
        }
        Ok(())
    }

    fn record_stale_tree_node(&mut self, stale: StaleTreeNode) -> Result<(), JmtStorageError> {
        // Prune staged tree nodes immediately from preceding_pending_state.
        let mut remove_queue = VecDeque::new();
        remove_queue.push_front(stale.as_node_key().clone());
        while let Some(key) = remove_queue.pop_front() {
            if let Some(node) = self.preceding_pending_state.remove(&key) {
                match node {
                    Node::Internal(node) => {
                        for (nibble, child) in node.into_children() {
                            remove_queue.push_back(key.gen_child_node_key(child.version, nibble));
                        }
                    },
                    Node::Leaf(_) | Node::Null => {},
                }
            }
        }

        self.new_stale_nodes.push(stale);
        Ok(())
    }
}
