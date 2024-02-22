//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};

use crate::{
    jellyfish::{JmtStorageError, Node, NodeKey},
    Version,
};

/// Implementers are able to read nodes from a tree store.
pub trait TreeStoreReader<P> {
    /// Gets node by key, if it exists.
    fn get_node(&self, key: &NodeKey) -> Result<Node<P>, JmtStorageError>;
}

/// Implementers are able to insert nodes to a tree store.
pub trait TreeStoreWriter<P> {
    /// Inserts the node under a new, unique key (i.e. never an update).
    fn insert_node(&mut self, key: NodeKey, node: Node<P>) -> Result<(), JmtStorageError>;

    /// Marks the given tree part for a (potential) future removal by an arbitrary external pruning
    /// process.
    fn record_stale_tree_node(&mut self, part: StaleTreeNode) -> Result<(), JmtStorageError>;
}

/// Implementers are able to read and write nodes to a tree store.
pub trait TreeStore<P>: TreeStoreReader<P> + TreeStoreWriter<P> {}
impl<P, S: TreeStoreReader<P> + TreeStoreWriter<P>> TreeStore<P> for S {}

/// A part of a tree that may become stale (i.e. need eventual pruning).
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum StaleTreeNode {
    /// A single node to be removed.
    Node(NodeKey),
    /// An entire subtree of descendants of a specific node (including itself).
    Subtree(NodeKey),
}

impl StaleTreeNode {
    pub fn into_node_key(self) -> NodeKey {
        match self {
            Self::Node(key) | Self::Subtree(key) => key,
        }
    }

    pub fn as_node_key(&self) -> &NodeKey {
        match self {
            Self::Node(key) | Self::Subtree(key) => key,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TreeNode {
    V1(Node<Version>),
}

impl TreeNode {
    pub fn new_latest(node: Node<Version>) -> Self {
        Self::new_v1(node)
    }

    pub fn new_v1(node: Node<Version>) -> Self {
        Self::V1(node)
    }

    pub fn as_node(&self) -> &Node<Version> {
        match self {
            Self::V1(node) => node,
        }
    }

    pub fn into_node(self) -> Node<Version> {
        match self {
            Self::V1(node) => node,
        }
    }
}
