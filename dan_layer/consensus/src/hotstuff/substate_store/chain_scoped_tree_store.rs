//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::ops::Deref;

use tari_dan_common_types::{optional::Optional, shard::Shard, Epoch};
use tari_dan_storage::{StateStoreReadTransaction, StateStoreWriteTransaction};
use tari_state_tree::{Node, NodeKey, StaleTreeNode, TreeStoreReader, TreeStoreWriter, Version};

/// Tree store that is scoped to a specific chain (epoch and shard)
#[derive(Debug)]
pub struct ChainScopedTreeStore<TTx> {
    epoch: Epoch,
    shard: Shard,
    tx: TTx,
}

impl<TTx> ChainScopedTreeStore<TTx> {
    pub fn new(epoch: Epoch, shard: Shard, tx: TTx) -> Self {
        Self { epoch, shard, tx }
    }
}

impl<TTx: Clone> ChainScopedTreeStore<TTx> {
    pub fn transaction(&self) -> TTx {
        self.tx.clone()
    }
}

impl<'a, TTx: StateStoreReadTransaction> TreeStoreReader<Version> for ChainScopedTreeStore<&'a TTx> {
    fn get_node(&self, key: &NodeKey) -> Result<Node<Version>, tari_state_tree::JmtStorageError> {
        self.tx
            .state_tree_nodes_get(self.epoch, self.shard, key)
            .optional()
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))?
            .ok_or_else(|| tari_state_tree::JmtStorageError::NotFound(key.clone()))
    }
}

impl<'a, TTx> TreeStoreReader<Version> for ChainScopedTreeStore<&'a mut TTx>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
{
    fn get_node(&self, key: &NodeKey) -> Result<Node<Version>, tari_state_tree::JmtStorageError> {
        self.tx
            .state_tree_nodes_get(self.epoch, self.shard, key)
            .optional()
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))?
            .ok_or_else(|| tari_state_tree::JmtStorageError::NotFound(key.clone()))
    }
}

impl<'a, TTx: StateStoreWriteTransaction> TreeStoreWriter<Version> for ChainScopedTreeStore<&'a mut TTx> {
    fn insert_node(&mut self, key: NodeKey, node: Node<Version>) -> Result<(), tari_state_tree::JmtStorageError> {
        self.tx
            .state_tree_nodes_insert(self.epoch, self.shard, key, node)
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))
    }

    fn record_stale_tree_node(&mut self, node: StaleTreeNode) -> Result<(), tari_state_tree::JmtStorageError> {
        self.tx
            .state_tree_nodes_mark_stale_tree_node(self.epoch, self.shard, node)
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))
    }
}
