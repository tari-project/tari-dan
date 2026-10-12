//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use helpers::assert_eq_debug;
use tari_ootle_common_types::{ShardGroup, optional::Optional, shard::Shard};
use tari_ootle_storage::{
    ShardScopedTreeStoreWriter,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};
use tari_state_store_rocksdb::DatabaseOptions;
use tari_state_tree::{NibblePath, Node, NodeKey, StaleTreeNode, StateTreePayload};

use crate::helpers::{create_rocksdb_with_opts, num_preshards};

#[test]
fn state_tree_rocksdb() {
    let (db, _tmp) = create_rocksdb_with_opts(DatabaseOptions::default().with_state_history_length(0));
    let timer = std::time::Instant::now();
    state_tree_operations(db, 10_000);
    println!("state_tree_rocksdb: 10_000 nodes: {:?}", timer.elapsed());
}

fn state_tree_operations(db: impl StateStore, num_nodes: usize) {
    const SHARD: Shard = Shard::first();

    // state_tree
    let nodes = gen_nodes(1, num_nodes / 2)
        .chain(gen_nodes(2, num_nodes / 2))
        .collect::<Vec<_>>();
    db.with_write_tx(|tx| {
        for (key, value) in &nodes {
            tx.state_tree_nodes_batch_insert(SHARD, &[(key.clone(), value.clone())])
                .unwrap();
        }
        Ok::<_, StorageError>(())
    })
    .unwrap();

    db.with_read_tx(|tx| {
        for (key, value) in &nodes {
            let res = tx.state_tree_nodes_get(SHARD, key).unwrap();
            assert_eq_debug(&res, value);
        }
        Ok::<_, StorageError>(())
    })
    .unwrap();

    db.with_write_tx(|tx| {
        tx.state_tree_nodes_record_stale_tree_nodes(
            SHARD,
            2,
            &nodes[..100]
                .iter()
                .map(|(k, _)| StaleTreeNode::Node(k.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        for shard in ShardGroup::all_shards(num_preshards()).shard_iter() {
            tx.state_tree_shard_versions_set(shard, 100).unwrap();
        }
        Ok::<_, StorageError>(())
    })
    .unwrap();

    let n = db
        .with_write_tx(|tx| tx.state_tree_nodes_clear_stale(num_preshards(), usize::MAX))
        .unwrap();
    // 100 nodes and the version's stale record
    assert_eq!(n, 101);
    db.with_read_tx(|tx| {
        // Stale nodes are gone
        for (key, _) in &nodes[..100] {
            let res = tx.state_tree_nodes_get(SHARD, key).optional().unwrap();
            assert!(res.is_none());
        }
        for (key, _) in &nodes[100..] {
            let res = tx.state_tree_nodes_get(SHARD, key).optional().unwrap();
            assert!(res.is_some());
        }

        for shard in ShardGroup::all_shards(num_preshards()).shard_iter() {
            let version = tx.state_tree_versions_get_latest(shard).unwrap().expect("version");
            assert_eq!(version, 100);
        }
        Ok::<_, StorageError>(())
    })
    .unwrap();
}

fn gen_nodes(version: u64, num: usize) -> impl Iterator<Item = (NodeKey, Node<StateTreePayload>)> {
    (0..num as u64).map(move |i| {
        let node = Node::Null;
        // No possibility of key collisions
        let path = NibblePath::new_even(&i.to_be_bytes()).unwrap();
        let node_key = NodeKey::new(version, path);
        (node_key, node)
    })
}

/// A call deletes whole stale versions until it reaches `max_deletes`, so repeated calls clear everything in bounded
/// transactions, each resuming at the first version the previous one left.
#[test]
fn clear_stale_is_bounded_per_call_and_resumes() {
    const SHARD: Shard = Shard::first();
    let (db, _tmp) = create_rocksdb_with_opts(DatabaseOptions::default().with_state_history_length(0));
    let per_version = 10usize;
    let all_nodes = (1u64..=5)
        .map(|v| gen_nodes(v, per_version).collect::<Vec<_>>())
        .collect::<Vec<_>>();

    db.with_write_tx(|tx| {
        for (v, nodes) in (1u64..).zip(&all_nodes) {
            tx.state_tree_nodes_batch_insert(SHARD, nodes).unwrap();
            tx.state_tree_nodes_record_stale_tree_nodes(
                SHARD,
                v,
                &nodes
                    .iter()
                    .map(|(k, _)| StaleTreeNode::Node(k.clone()))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        }
        tx.state_tree_shard_versions_set(SHARD, 5).unwrap();
        Ok::<_, StorageError>(())
    })
    .unwrap();

    let clear = |max_deletes| {
        db.with_write_tx(|tx| tx.state_tree_nodes_clear_stale(num_preshards(), max_deletes))
            .unwrap()
    };
    let is_present = |nodes: &[(NodeKey, Node<StateTreePayload>)]| {
        db.with_read_tx(|tx| {
            let present = nodes
                .iter()
                .map(|(key, _)| tx.state_tree_nodes_get(SHARD, key).optional().map(|n| n.is_some()))
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<_, StorageError>(present)
        })
        .unwrap()
    };

    // Each version is 11 rows: 10 nodes and its stale record. A budget of 15 stops after the second version.
    assert_eq!(clear(15), 22);
    assert!(all_nodes[..2].iter().all(|nodes| !is_present(nodes).contains(&true)));
    assert!(all_nodes[2..].iter().all(|nodes| !is_present(nodes).contains(&false)));

    assert_eq!(clear(15), 22);
    assert!(!is_present(&all_nodes[3]).contains(&true));
    // The last version fits under the budget, which signals nothing is left.
    assert_eq!(clear(15), 11);
    assert_eq!(clear(15), 0);
    assert!(all_nodes.iter().all(|nodes| !is_present(nodes).contains(&true)));
}

#[test]
fn truncate_to_version_removes_newer_versions_and_resets_pointer() {
    const SHARD: Shard = Shard::first();
    let (db, _tmp) = create_rocksdb_with_opts(DatabaseOptions::default().with_state_history_length(0));

    // Seed five versions' worth of nodes (10 per version).
    let per_version = 10usize;
    #[expect(clippy::type_complexity)]
    let all_nodes: Vec<(u64, Vec<(NodeKey, Node<StateTreePayload>)>)> =
        (1u64..=5).map(|v| (v, gen_nodes(v, per_version).collect())).collect();

    db.with_write_tx(|tx| {
        for (_, nodes) in &all_nodes {
            tx.state_tree_nodes_batch_insert(SHARD, nodes).unwrap();
        }
        // Record stale markers at versions 3, 4, 5 — these should be truncated.
        for v in [3, 4, 5] {
            tx.state_tree_nodes_record_stale_tree_nodes(SHARD, v, &[StaleTreeNode::Node(all_nodes[0].1[0].0.clone())])
                .unwrap();
        }
        // And a stale marker at version 2 that should survive.
        tx.state_tree_nodes_record_stale_tree_nodes(SHARD, 2, &[StaleTreeNode::Node(all_nodes[0].1[0].0.clone())])
            .unwrap();
        tx.state_tree_shard_versions_set(SHARD, 5).unwrap();
        Ok::<_, StorageError>(())
    })
    .unwrap();

    let stats = db
        .with_write_tx(|tx| tx.state_tree_truncate_to_version(SHARD, 2))
        .unwrap();

    // 3 versions × 10 nodes per version = 30 nodes deleted.
    assert_eq!(stats.nodes_deleted, 30);
    // Stale records at versions 3, 4, 5 deleted.
    assert_eq!(stats.stale_records_deleted, 3);

    db.with_read_tx(|tx| {
        // Versions 1 and 2 survive.
        for (v, nodes) in &all_nodes[..2] {
            for (key, _) in nodes {
                let got = tx.state_tree_nodes_get(SHARD, key).optional().unwrap();
                assert!(got.is_some(), "node at v{v} was unexpectedly deleted");
            }
        }
        // Versions 3, 4, 5 are gone.
        for (v, nodes) in &all_nodes[2..] {
            for (key, _) in nodes {
                let got = tx.state_tree_nodes_get(SHARD, key).optional().unwrap();
                assert!(got.is_none(), "node at v{v} survived truncation");
            }
        }

        // Version pointer reset to 2.
        let latest = tx.state_tree_versions_get_latest(SHARD).unwrap();
        assert_eq!(latest, Some(2));
        Ok::<_, StorageError>(())
    })
    .unwrap();
}

#[test]
fn truncate_to_version_is_shard_scoped() {
    let (db, _tmp) = create_rocksdb_with_opts(DatabaseOptions::default().with_state_history_length(0));
    let shard_a = Shard::first();
    let shard_b = Shard::from_u32(42);

    let nodes_v3 = gen_nodes(3, 5).collect::<Vec<_>>();

    db.with_write_tx(|tx| {
        tx.state_tree_nodes_batch_insert(shard_a, &nodes_v3).unwrap();
        tx.state_tree_nodes_batch_insert(shard_b, &nodes_v3).unwrap();
        tx.state_tree_shard_versions_set(shard_a, 3).unwrap();
        tx.state_tree_shard_versions_set(shard_b, 3).unwrap();
        Ok::<_, StorageError>(())
    })
    .unwrap();

    let stats = db
        .with_write_tx(|tx| tx.state_tree_truncate_to_version(shard_a, 1))
        .unwrap();
    assert_eq!(stats.nodes_deleted, 5);

    db.with_read_tx(|tx| {
        // shard_b untouched.
        for (key, _) in &nodes_v3 {
            assert!(tx.state_tree_nodes_get(shard_b, key).optional().unwrap().is_some());
        }
        assert_eq!(tx.state_tree_versions_get_latest(shard_b).unwrap(), Some(3));
        // shard_a truncated. No nodes survive at or below the target version, so the version
        // pointer is removed and the shard reads as an empty tree.
        for (key, _) in &nodes_v3 {
            assert!(tx.state_tree_nodes_get(shard_a, key).optional().unwrap().is_none());
        }
        assert_eq!(tx.state_tree_versions_get_latest(shard_a).unwrap(), None);
        Ok::<_, StorageError>(())
    })
    .unwrap();
}

#[test]
fn truncate_to_version_zero_keeps_genesis_v0_pointer() {
    // Genesis substates are bootstrapped into the tree at version 0, so a shard rolled back to a
    // checkpoint that recorded state_version 0 must keep its v0 nodes and a version pointer of 0.
    let (db, _tmp) = create_rocksdb_with_opts(DatabaseOptions::default().with_state_history_length(0));
    let genesis_shard = Shard::first();
    let empty_shard = Shard::from_u32(42);

    let nodes_v0 = gen_nodes(0, 5).collect::<Vec<_>>();
    let nodes_v1 = gen_nodes(1, 5).collect::<Vec<_>>();
    let nodes_v2 = gen_nodes(2, 5).collect::<Vec<_>>();

    db.with_write_tx(|tx| {
        // genesis_shard: bootstrapped at v0, then two consensus commits.
        tx.state_tree_nodes_batch_insert(genesis_shard, &nodes_v0).unwrap();
        tx.state_tree_nodes_batch_insert(genesis_shard, &nodes_v1).unwrap();
        tx.state_tree_nodes_batch_insert(genesis_shard, &nodes_v2).unwrap();
        tx.state_tree_shard_versions_set(genesis_shard, 2).unwrap();
        // empty_shard: first state committed at v1 (no genesis substates).
        tx.state_tree_nodes_batch_insert(empty_shard, &nodes_v1).unwrap();
        tx.state_tree_shard_versions_set(empty_shard, 1).unwrap();
        Ok::<_, StorageError>(())
    })
    .unwrap();

    db.with_write_tx(|tx| tx.state_tree_truncate_to_version(genesis_shard, 0))
        .unwrap();
    db.with_write_tx(|tx| tx.state_tree_truncate_to_version(empty_shard, 0))
        .unwrap();

    db.with_read_tx(|tx| {
        // genesis_shard: v0 nodes survive and the pointer stays at 0.
        for (key, _) in &nodes_v0 {
            assert!(
                tx.state_tree_nodes_get(genesis_shard, key)
                    .optional()
                    .unwrap()
                    .is_some()
            );
        }
        for (key, _) in nodes_v1.iter().chain(&nodes_v2) {
            assert!(
                tx.state_tree_nodes_get(genesis_shard, key)
                    .optional()
                    .unwrap()
                    .is_none()
            );
        }
        assert_eq!(tx.state_tree_versions_get_latest(genesis_shard).unwrap(), Some(0));

        // empty_shard: nothing survives at or below v0, so the pointer is removed.
        for (key, _) in &nodes_v1 {
            assert!(tx.state_tree_nodes_get(empty_shard, key).optional().unwrap().is_none());
        }
        assert_eq!(tx.state_tree_versions_get_latest(empty_shard).unwrap(), None);
        Ok::<_, StorageError>(())
    })
    .unwrap();
}

#[test]
fn set_state_version_refuses_to_rewrite_a_committed_version() {
    const SHARD: Shard = Shard::first();
    let (db, _tmp) = create_rocksdb_with_opts(DatabaseOptions::default());

    db.with_write_tx(|tx| {
        let mut store = ShardScopedTreeStoreWriter::new(tx, SHARD);
        store.set_state_version(1).unwrap();
        Ok::<_, StorageError>(())
    })
    .unwrap();

    // Writing v1 again would put new nodes on the keys the same write records as stale at v1, leaving
    // the tree pointing at nodes the stale-node GC is free to delete.
    db.with_write_tx(|tx| {
        let mut store = ShardScopedTreeStoreWriter::new(tx, SHARD);
        let err = store.set_state_version(1).unwrap_err();
        assert!(
            err.to_string().contains("must be greater than the current version"),
            "{err}"
        );
        let err = store.set_state_version(0).unwrap_err();
        assert!(
            err.to_string().contains("must be greater than the current version"),
            "{err}"
        );
        store.set_state_version(2).unwrap();
        Ok::<_, StorageError>(())
    })
    .unwrap();

    db.with_read_tx(|tx| {
        assert_eq!(tx.state_tree_versions_get_latest(SHARD).unwrap(), Some(2));
        Ok::<_, StorageError>(())
    })
    .unwrap();
}
