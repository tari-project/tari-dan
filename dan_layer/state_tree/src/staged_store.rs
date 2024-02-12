//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, VecDeque};

use crate::{
    JmtStorageError,
    Node,
    NodeKey,
    StaleTreeNode,
    StateHashTreeDiff,
    TreeStoreReader,
    TreeStoreWriter,
    Version,
};

pub struct StagedTreeStore<'s, S> {
    readable_store: &'s S,
    preceding_pending_state: HashMap<NodeKey, Node<Version>>,
    new_tree_nodes: HashMap<NodeKey, Node<Version>>,
    new_stale_nodes: Vec<StaleTreeNode>,
}

impl<'s, S: TreeStoreReader<Version>> StagedTreeStore<'s, S> {
    pub fn new(readable_store: &'s S) -> Self {
        Self {
            readable_store,
            preceding_pending_state: HashMap::new(),
            new_tree_nodes: HashMap::new(),
            new_stale_nodes: Vec::new(),
        }
    }

    pub fn apply_ordered_diffs<I: IntoIterator<Item = StateHashTreeDiff>>(&mut self, diffs: I) {
        for (key, node) in diffs.into_iter().flat_map(|diff| diff.new_nodes) {
            self.preceding_pending_state.insert(key, node);
        }
    }

    pub fn into_diff(self) -> StateHashTreeDiff {
        StateHashTreeDiff {
            new_nodes: self.new_tree_nodes.into_iter().collect(),
            stale_tree_nodes: self.new_stale_nodes,
        }
    }
}

impl<'s, S: TreeStoreReader<Version>> TreeStoreReader<Version> for StagedTreeStore<'s, S> {
    fn get_node(&self, key: &NodeKey) -> Result<Node<Version>, JmtStorageError> {
        if let Some(node) = self.new_tree_nodes.get(key).cloned() {
            return Ok(node);
        }
        if let Some(node) = self.preceding_pending_state.get(key).cloned() {
            return Ok(node);
        }

        self.readable_store.get_node(key)
    }
}

impl<'s, S> TreeStoreWriter<Version> for StagedTreeStore<'s, S> {
    fn insert_node(&mut self, key: NodeKey, node: Node<Version>) -> Result<(), JmtStorageError> {
        self.new_tree_nodes.insert(key, node);
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
