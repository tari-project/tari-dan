//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Version 3 stores state tree nodes, stale node lists and pending state tree diffs in a native minicbor encoding in
//! place of version 2's serde-bridged one. This migration rewrites every such value in place.

use log::*;
use tari_ootle_common_types::NodeAddressable;
use tari_ootle_storage::{Ordering, StateStoreWriteTransaction, consensus_models::PendingShardStateTreeDiff};
use tari_state_store_rocksdb::{
    column_families::{
        pending_state_tree_diff::legacy::PendingStateTreeDiffRecordV2Cf,
        state_tree::{StateTreeCf, StateTreeStaleNodesCf, legacy},
    },
    writer::RocksDbStateStoreWriteTransaction,
};

const LOG_TARGET: &str = "tari::validator_node::migrations::v3";

/// Nodes are read this many at a time.
const NODE_CHUNK_SIZE: usize = 10_000;

pub fn migrate<TAddr: NodeAddressable + 'static>(
    tx: &mut RocksDbStateStoreWriteTransaction<'_, TAddr>,
) -> anyhow::Result<()> {
    const OPERATION: &str = "migrate_v3";

    let num_nodes = {
        let db = tx.db();
        let legacy_nodes = db.cf(legacy::StateTreeCf)?;
        let nodes = db.cf(StateTreeCf)?;
        let keys = legacy_nodes
            .key_iterator(Ordering::Ascending, OPERATION)
            .collect::<Result<Vec<_>, _>>()?;
        for chunk in keys.chunks(NODE_CHUNK_SIZE) {
            let values = legacy_nodes.multi_get_exact(chunk.iter(), OPERATION)?;
            for (key, node) in chunk.iter().zip(values) {
                nodes.put(key, &node, OPERATION)?;
            }
        }
        keys.len()
    };

    let num_stale_lists = {
        let db = tx.db();
        let stale_lists = db
            .cf(legacy::StateTreeStaleNodesCf)?
            .iterator(Ordering::Ascending, OPERATION)
            .collect::<Result<Vec<_>, _>>()?;
        let stale_nodes = db.cf(StateTreeStaleNodesCf)?;
        for (key, list) in &stale_lists {
            stale_nodes.put(key, list, OPERATION)?;
        }
        stale_lists.len()
    };

    let records = tx
        .db()
        .cf(PendingStateTreeDiffRecordV2Cf)?
        .iterator(Ordering::Ascending, OPERATION)
        .collect::<Result<Vec<_>, _>>()?;
    let num_records = records.len();
    for (block_id, record) in records {
        let diffs = record
            .into_iter()
            .map(|legacy| (legacy.shard, PendingShardStateTreeDiff::from(legacy.diff)))
            .collect::<Vec<_>>();
        tx.pending_state_tree_diffs_insert_all(&block_id, diffs.iter().map(|(shard, diff)| (shard, diff)))?;
    }

    info!(
        target: LOG_TARGET,
        "Re-encoded {num_nodes} state tree node(s), {num_stale_lists} stale node list(s) and the pending state tree \
         diffs of {num_records} block(s)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use tari_consensus_types::BlockId;
    use tari_ootle_common_types::{SubstateAddress, shard::Shard};
    use tari_ootle_storage::{StateStore, StorageError};
    use tari_state_store_rocksdb::{
        DatabaseOptions,
        RocksDbStateStore,
        codecs::ByteColumn,
        column_families::{
            bookkeeping::DatabaseMigrationVersion,
            pending_state_tree_diff::legacy::{LegacyPendingShardStateTreeDiff, LegacyShardStateTreeDiff},
        },
    };
    use tari_state_tree::{
        JellyfishMerkleTree,
        JmtHashScheme,
        LeafKey,
        Node,
        NodeKey,
        StaleTreeNode,
        StateHashTreeDiff,
        TreeHash,
        memory_store::MemoryTreeStore,
    };

    use super::*;

    fn tree_diff() -> StateHashTreeDiff<SubstateAddress> {
        let store = MemoryTreeStore::new();
        let jmt = JellyfishMerkleTree::new(&store, JmtHashScheme::V1);
        let changes = (0..=255u8).map(|i| {
            let hash = TreeHash::new([i; 32]);
            (
                LeafKey::new(hash),
                Some((hash, SubstateAddress::from_bytes(&[i; 40]).unwrap())),
            )
        });
        let (_, batch) = jmt.batch_put_value_set(changes, None, 1).unwrap();
        let mut diff = StateHashTreeDiff::from(batch);
        diff.stale_tree_nodes
            .push(StaleTreeNode::Subtree(NodeKey::new_empty_path(0)));
        diff
    }

    /// Every state tree node, stale node list and pending tree diff written in version 2's encoding reads back the
    /// same in version 3's, and the pending diffs reach the in-memory table. A version 2 database opens before the
    /// migration, so the store must not read its pending diff records in the new encoding.
    #[test]
    fn version_2_tree_values_are_re_encoded() {
        const OPERATION: &str = "test";
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("rocksdb");
        let shard = Shard::from(5u32);
        let block_id = BlockId::new([7u8; 32]);
        let diff = tree_diff();
        let stale = vec![StaleTreeNode::Node(NodeKey::new_empty_path(3))];

        let db = RocksDbStateStore::<String>::open(&path, DatabaseOptions::default()).unwrap();
        db.with_write_tx(|tx| {
            let db = tx.db();
            for (key, node) in &diff.new_nodes {
                db.cf(legacy::StateTreeCf)?
                    .put(&(shard, key.clone()), node, OPERATION)?;
            }
            db.cf(legacy::StateTreeStaleNodesCf)?
                .put(&(shard, 4), &stale, OPERATION)?;
            let record = vec![LegacyShardStateTreeDiff {
                shard,
                diff: LegacyPendingShardStateTreeDiff {
                    version: 6,
                    diff: diff.clone(),
                },
            }];
            db.cf(PendingStateTreeDiffRecordV2Cf)?
                .put(&block_id, &record, OPERATION)?;
            db.cf(DatabaseMigrationVersion)?.put(&ByteColumn, &2, OPERATION)?;
            Ok::<_, StorageError>(())
        })
        .unwrap();
        drop(db);

        let db = RocksDbStateStore::<String>::open(&path, DatabaseOptions::default()).unwrap();
        db.with_write_tx(|tx| {
            migrate(tx)?;
            tx.db().cf(DatabaseMigrationVersion)?.put(&ByteColumn, &3, OPERATION)?;
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();
        drop(db);

        let db = RocksDbStateStore::<String>::open(&path, DatabaseOptions::default()).unwrap();
        db.with_write_tx(|tx| {
            let ctx = tx.db();
            for (key, node) in &diff.new_nodes {
                let migrated: Node<_> = ctx.cf(StateTreeCf)?.get(&(shard, key.clone()), OPERATION)?;
                assert_eq!(&migrated, node);
            }
            assert_eq!(ctx.cf(StateTreeStaleNodesCf)?.get(&(shard, 4), OPERATION)?, stale);

            let pending = tx.pending_state_tree_diffs_remove_and_return_by_block(&block_id)?;
            assert_eq!(pending[&shard].len(), 1);
            let migrated = &pending[&shard][0];
            assert_eq!(migrated.version, 6);
            assert_eq!(migrated.diff.new_nodes, diff.new_nodes);
            assert_eq!(migrated.diff.stale_tree_nodes, diff.stale_tree_nodes);
            let (key, node) = &diff.new_nodes[0];
            assert_eq!(migrated.diff.get_node(key), Some(node));
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();
    }
}
