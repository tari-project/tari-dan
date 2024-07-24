//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl};
use tari_dan_common_types::{shard::Shard, Epoch};
use tari_state_tree::{Node, NodeKey, StaleTreeNode, TreeNode, TreeStoreReader, TreeStoreWriter, Version};

use crate::{reader::SqliteStateStoreReadTransaction, writer::SqliteStateStoreWriteTransaction};

pub struct SqliteChainScopedTreeStore<TTx> {
    epoch: Epoch,
    shard: Shard,
    db_tx: TTx,
}

impl<TTx> SqliteChainScopedTreeStore<TTx> {
    pub fn new(epoch: Epoch, shard: Shard, db_tx: TTx) -> Self {
        Self { epoch, shard, db_tx }
    }

    fn db_epoch(&self) -> i64 {
        self.epoch.as_u64() as i64
    }

    fn db_shard(&self) -> i32 {
        self.shard.as_u32() as i32
    }
}

impl<'a, TAddr> TreeStoreReader<Version>
    for SqliteChainScopedTreeStore<&'a SqliteStateStoreReadTransaction<'a, TAddr>>
{
    fn get_node(&self, key: &NodeKey) -> Result<Node<Version>, tari_state_tree::JmtStorageError> {
        use crate::schema::state_tree;

        let node = state_tree::table
            .select(state_tree::node)
            .filter(state_tree::epoch.eq(self.db_epoch()))
            .filter(state_tree::shard.eq(self.db_shard()))
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

impl<'a, TAddr> TreeStoreReader<Version>
    for SqliteChainScopedTreeStore<&'a mut SqliteStateStoreWriteTransaction<'a, TAddr>>
{
    fn get_node(&self, key: &NodeKey) -> Result<Node<Version>, tari_state_tree::JmtStorageError> {
        SqliteChainScopedTreeStore::new(self.epoch, self.shard, &**self.db_tx).get_node(key)
    }
}

impl<'a, TAddr> TreeStoreWriter<Version>
    for SqliteChainScopedTreeStore<&'a mut SqliteStateStoreWriteTransaction<'a, TAddr>>
{
    fn insert_node(&mut self, key: NodeKey, node: Node<Version>) -> Result<(), tari_state_tree::JmtStorageError> {
        use crate::schema::state_tree;

        let node = TreeNode::new_latest(node);
        let node = serde_json::to_string(&node)
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))?;

        let values = (
            state_tree::epoch.eq(self.db_epoch()),
            state_tree::shard.eq(self.db_shard()),
            state_tree::key.eq(key.to_string()),
            state_tree::node.eq(node),
        );
        diesel::insert_into(state_tree::table)
            .values(&values)
            .on_conflict((state_tree::epoch, state_tree::shard, state_tree::key))
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
            .filter(state_tree::epoch.eq(self.db_epoch()))
            .filter(state_tree::shard.eq(self.db_shard()))
            .filter(state_tree::key.eq(key.to_string()))
            .set(state_tree::is_stale.eq(true))
            .execute(self.connection())
            .optional()
            .map_err(|e| tari_state_tree::JmtStorageError::UnexpectedError(e.to_string()))?
            .ok_or_else(|| tari_state_tree::JmtStorageError::NotFound(key.clone()))?;

        Ok(())
    }
}
