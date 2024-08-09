//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::ops::Deref;

use tari_dan_common_types::{optional::Optional, shard::Shard};
use tari_dan_storage::{StateStoreReadTransaction, StateStoreWriteTransaction};
use tari_state_tree::{Node, NodeKey, StaleTreeNode, TreeStoreReader, TreeStoreWriter, Version};

/// Tree store that is scoped to a specific shard
#[derive(Debug)]
pub struct ShardScopedTreeStoreReader<'a, TTx> {
    shard: Shard,
    tx: &'a TTx,
}

impl<'a, TTx> ShardScopedTreeStoreReader<'a, TTx> {
    pub fn new(tx: &'a TTx, shard: Shard) -> Self {
        Self { shard, tx }
    }
}

impl<'a, TTx: StateStoreReadTransaction> TreeStoreReader<Version> for ShardScopedTreeStoreReader<'a, TTx> {
    fn get_node(&self, key: &NodeKey) -> Result<Node<Version>, tari_state_tree::JmtStorageError> {
        self.tx
            .state_tree_nodes_get(self.shard, key)
            .optional()
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))?
            .ok_or_else(|| tari_state_tree::JmtStorageError::NotFound(key.clone()))
    }
}

#[derive(Debug)]
pub struct ShardScopedTreeStoreWriter<'a, TTx> {
    shard: Shard,
    tx: &'a mut TTx,
}

impl<'a, TTx: StateStoreWriteTransaction> ShardScopedTreeStoreWriter<'a, TTx> {
    pub fn new(tx: &'a mut TTx, shard: Shard) -> Self {
        Self { shard, tx }
    }

    pub fn set_version(&mut self, version: Version) -> Result<(), tari_state_tree::JmtStorageError> {
        self.tx
            .state_tree_shard_versions_set(self.shard, version)
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))
    }

    pub fn transaction(&mut self) -> &mut TTx {
        self.tx
    }
}

impl<'a, TTx> TreeStoreReader<Version> for ShardScopedTreeStoreWriter<'a, TTx>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
{
    fn get_node(&self, key: &NodeKey) -> Result<Node<Version>, tari_state_tree::JmtStorageError> {
        self.tx
            .state_tree_nodes_get(self.shard, key)
            .optional()
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))?
            .ok_or_else(|| tari_state_tree::JmtStorageError::NotFound(key.clone()))
    }
}

impl<'a, TTx: StateStoreWriteTransaction> TreeStoreWriter<Version> for ShardScopedTreeStoreWriter<'a, TTx> {
    fn insert_node(&mut self, key: NodeKey, node: Node<Version>) -> Result<(), tari_state_tree::JmtStorageError> {
        self.tx
            .state_tree_nodes_insert(self.shard, key, node)
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))
    }

    fn record_stale_tree_node(&mut self, node: StaleTreeNode) -> Result<(), tari_state_tree::JmtStorageError> {
        self.tx
            .state_tree_nodes_record_stale_tree_node(self.shard, node)
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))
    }
}
