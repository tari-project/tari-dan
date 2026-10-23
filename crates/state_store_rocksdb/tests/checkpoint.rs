//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use helpers::{commit_chain, create_block, create_chain, create_rocksdb};
use tari_ootle_storage::{StateStore, StateStoreReadTransaction};
use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};

#[test]
fn checkpoint_holds_the_state_at_the_time_it_was_taken() {
    let (db, tmp) = create_rocksdb();
    let chain = create_chain(5);
    db.with_write_tx(|tx| {
        commit_chain(tx, &chain);
        Ok::<_, tari_ootle_storage::StorageError>(())
    })
    .unwrap();

    let checkpoint_dir = tmp.path().join("checkpoint");
    db.create_checkpoint(&checkpoint_dir).unwrap();

    let later = create_block(chain.last());
    db.with_write_tx(|tx| later.insert(tx)).unwrap();

    let checkpoint = RocksDbStateStore::<String>::open(&checkpoint_dir, DatabaseOptions::default()).unwrap();
    let tx = checkpoint.create_read_tx().unwrap();
    for block in &chain {
        assert!(tx.blocks_exists(block.id()).unwrap());
    }
    assert!(!tx.blocks_exists(later.id()).unwrap());
    drop(tx);

    let tx = db.create_read_tx().unwrap();
    assert!(tx.blocks_exists(later.id()).unwrap());
}

#[test]
fn checkpoint_into_an_existing_directory_fails() {
    let (db, tmp) = create_rocksdb();
    let checkpoint_dir = tmp.path().join("checkpoint");
    std::fs::create_dir_all(&checkpoint_dir).unwrap();

    assert!(db.create_checkpoint(&checkpoint_dir).is_err());
}
