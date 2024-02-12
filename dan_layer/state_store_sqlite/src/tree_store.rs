//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::Deref;

use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl};
use tari_state_tree::{Node, NodeKey, StaleTreeNode, TreeNode, TreeStoreReader, TreeStoreWriter, Version};

use crate::{reader::SqliteStateStoreReadTransaction, writer::SqliteStateStoreWriteTransaction};

impl<'a, TAddr> TreeStoreReader<Version> for SqliteStateStoreReadTransaction<'a, TAddr> {
    fn get_node(&self, key: &NodeKey) -> Result<Node<Version>, tari_state_tree::JmtStorageError> {
        use crate::schema::state_tree;

        let node = state_tree::table
            .select(state_tree::node)
            .filter(state_tree::key.eq(key.to_string()))
            .filter(state_tree::is_stale.eq(false))
            .first::<String>(self.connection())
            .optional()
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))?
            .ok_or_else(|| tari_state_tree::JmtStorageError::NotFound(key.clone()))?;

        let node = serde_json::from_str::<TreeNode>(&node)
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))?;

        Ok(node.into_node())
    }
}

impl<'a, TAddr> TreeStoreReader<Version> for SqliteStateStoreWriteTransaction<'a, TAddr> {
    fn get_node(&self, key: &NodeKey) -> Result<Node<Version>, tari_state_tree::JmtStorageError> {
        self.deref().get_node(key)
    }
}

impl<'a, TAddr> TreeStoreWriter<Version> for SqliteStateStoreWriteTransaction<'a, TAddr> {
    fn insert_node(&mut self, key: NodeKey, node: Node<Version>) -> Result<(), tari_state_tree::JmtStorageError> {
        use crate::schema::state_tree;

        let node = TreeNode::new_latest(node);
        let node = serde_json::to_string(&node)
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))?;

        let values = (state_tree::key.eq(key.to_string()), state_tree::node.eq(node));
        diesel::insert_into(state_tree::table)
            .values(&values)
            .on_conflict(state_tree::key)
            .do_update()
            .set(values.clone())
            .execute(self.connection())
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))?;

        Ok(())
    }

    fn record_stale_tree_node(&mut self, node: StaleTreeNode) -> Result<(), tari_state_tree::JmtStorageError> {
        use crate::schema::state_tree;
        let key = node.as_node_key();
        diesel::update(state_tree::table)
            .filter(state_tree::key.eq(key.to_string()))
            .set(state_tree::is_stale.eq(true))
            .execute(self.connection())
            .optional()
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))?
            .ok_or_else(|| tari_state_tree::JmtStorageError::NotFound(key.clone()))?;

        Ok(())
    }
}
