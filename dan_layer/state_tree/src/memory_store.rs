//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, fmt};

use crate::jellyfish::{
    JmtStorageError,
    Node,
    NodeKey,
    StaleTreeNode,
    TreeNode,
    TreeStoreReader,
    TreeStoreWriter,
    Version,
};

#[derive(Debug, Default)]
pub struct MemoryTreeStore {
    pub nodes: HashMap<NodeKey, TreeNode>,
    pub stale_nodes: Vec<StaleTreeNode>,
}

impl MemoryTreeStore {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            stale_nodes: Vec::new(),
        }
    }

    pub fn clear_stale_nodes(&mut self) {
        for stale in self.stale_nodes.drain(..) {
            self.nodes.remove(stale.as_node_key());
        }
    }
}

impl TreeStoreReader<Version> for MemoryTreeStore {
    fn get_node(&self, key: &NodeKey) -> Result<Node<Version>, JmtStorageError> {
        self.nodes
            .get(key)
            .map(|node| node.clone().into_node())
            .ok_or_else(|| JmtStorageError::NotFound(key.clone()))
    }
}

impl TreeStoreWriter<Version> for MemoryTreeStore {
    fn insert_node(&mut self, key: NodeKey, node: Node<Version>) -> Result<(), JmtStorageError> {
        let node = TreeNode::new_latest(node);
        self.nodes.insert(key, node);
        Ok(())
    }

    fn record_stale_tree_node(&mut self, stale: StaleTreeNode) -> Result<(), JmtStorageError> {
        self.stale_nodes.push(stale);
        Ok(())
    }
}

impl fmt::Display for MemoryTreeStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "MemoryTreeStore")?;
        writeln!(f, "  Nodes:")?;
        let mut store = self.nodes.iter().collect::<Vec<_>>();
        store.sort_by_key(|(key, _)| *key);
        for (key, node) in store {
            writeln!(f, "    {}: {:?}", key, node)?;
        }
        writeln!(f, "  Stale Nodes:")?;
        for stale in &self.stale_nodes {
            writeln!(f, "    {}", stale.as_node_key())?;
        }
        Ok(())
    }
}
