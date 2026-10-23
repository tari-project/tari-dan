//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use helpers::{commit_chain, create_block, create_chain, create_rocksdb};
use tari_consensus_types::PcId;
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{BookkeepingModel, PendingShardStateTreeDiff},
};
use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
use tari_state_tree::{Node, NodeKey, StateHashTreeDiff, TransitionAccumulator};

#[test]
fn pending_state_tree_diff_rocksdb() {
    let (db, _tmp) = create_rocksdb();
    pending_state_tree_diff_operations(db);
}

fn pending_state_tree_diff_operations(db: impl StateStore) {
    let mut tx = db.create_write_tx().unwrap();

    // add some (committed) blocks to the database
    let mut genesis = create_block(None);
    genesis.set_commit_qc(PcId::zero());
    genesis.insert(&mut tx).unwrap();
    tx.blocks_set_qcs(genesis.id(), Some(&PcId::zero()), Some(&PcId::zero()))
        .unwrap();
    tx.proposal_certificates_save(genesis.justify()).unwrap();
    genesis.as_locked().set(&mut tx).unwrap();
    genesis.as_leaf().set(&mut tx).unwrap();

    let mut block_1 = create_block(Some(&genesis));
    block_1.set_commit_qc(PcId::zero());
    block_1.insert(&mut tx).unwrap();

    let block_2 = create_block(Some(&block_1));
    block_2.insert(&mut tx).unwrap();

    let block_3 = create_block(Some(&block_2));
    block_3.insert(&mut tx).unwrap();

    // pending_state_tree_diffs_insert_all
    let shard = block_2.shard_group().shard_iter().next().unwrap();
    let diff = PendingShardStateTreeDiff::new(0, TransitionAccumulator::zero(), StateHashTreeDiff::new());
    tx.pending_state_tree_diffs_insert_all(block_2.id(), [(&shard, &diff)])
        .unwrap();

    // pending_state_tree_diffs_get_all_up_to_commit_block
    let res = tx
        .pending_state_tree_diffs_get_all_up_to_commit_block(block_3.id())
        .unwrap();
    assert_eq!(res.len(), 1);

    // pending_state_tree_diffs_remove_and_return_by_block
    let res = tx
        .pending_state_tree_diffs_remove_and_return_by_block(block_2.id())
        .unwrap();
    assert_eq!(res.len(), 1);
    let res = tx
        .pending_state_tree_diffs_get_all_up_to_commit_block(block_3.id())
        .unwrap();
    assert_eq!(res.len(), 0);

    tx.rollback().unwrap();
}

fn diff_with_root(version: u64) -> PendingShardStateTreeDiff {
    PendingShardStateTreeDiff::new(version, TransitionAccumulator::zero(), StateHashTreeDiff {
        new_nodes: vec![(NodeKey::new_empty_path(version), Node::Null)],
        stale_tree_nodes: vec![],
    })
}

/// A block's pending state tree diffs, one per shard, survive reopening the store as one record, still indexed by
/// node key, and removing them is persisted.
#[test]
fn pending_state_tree_diffs_survive_reopening_the_store() {
    let (db, tmp) = create_rocksdb();
    let chain = create_chain(10);
    let block8 = &chain[8];
    let shards = block8.shard_group().shard_iter().take(3).collect::<Vec<_>>();
    let diffs = shards
        .iter()
        .enumerate()
        .map(|(i, shard)| (*shard, diff_with_root(i as u64)))
        .collect::<Vec<_>>();
    db.with_write_tx(|tx| {
        commit_chain(tx, &chain);
        tx.pending_state_tree_diffs_insert_all(block8.id(), diffs.iter().map(|(shard, diff)| (shard, diff)))
    })
    .unwrap();

    drop(db);
    let db = RocksDbStateStore::<String>::open(tmp.path().join("rocksdb"), DatabaseOptions::default()).unwrap();
    let res = db
        .with_write_tx(|tx| tx.pending_state_tree_diffs_remove_and_return_by_block(block8.id()))
        .unwrap();
    assert_eq!(res.len(), 3);
    for (i, shard) in shards.iter().enumerate() {
        assert_eq!(res[shard].len(), 1);
        assert_eq!(res[shard][0].version, i as u64);
        assert_eq!(
            res[shard][0].diff.get_node(&NodeKey::new_empty_path(i as u64)),
            Some(&Node::Null)
        );
    }

    drop(db);
    let db = RocksDbStateStore::<String>::open(tmp.path().join("rocksdb"), DatabaseOptions::default()).unwrap();
    let res = db
        .with_write_tx(|tx| tx.pending_state_tree_diffs_remove_and_return_by_block(block8.id()))
        .unwrap();
    assert!(res.is_empty());
}
