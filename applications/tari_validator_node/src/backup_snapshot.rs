//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Point-in-time copies of the validator's state, taken while it runs, for an operator to ship off the host.
//!
//! A snapshot is a directory holding a RocksDB checkpoint of the state store (`rocksdb/`) and a copy of the global
//! database (`global_storage.sqlite`). Restoring one means stopping the node, putting those two in place of the
//! data directory's `rocksdb/` and `global_storage.sqlite`, and starting it again; the node syncs whatever it missed
//! from its peers.
//!
//! The checkpoint hard-links the store's SST files, so it pins their disk space for as long as it exists. Only one
//! snapshot may exist at a time: deleting it once it has been copied off the host is the caller's job, and a
//! snapshot left behind makes the next request fail instead of quietly accumulating.

use std::{
    fs,
    io,
    path::{Path, PathBuf},
};

use tari_ootle_p2p::PeerAddress;
use tari_ootle_storage::{StorageError, global::GlobalDb};
use tari_ootle_storage_sqlite::{error::SqliteStorageError, global::SqliteGlobalDbAdapter};
use time::OffsetDateTime;

use crate::consensus::spec::ValidatorNodeStateStore;

pub const STATE_STORE_DIR: &str = "rocksdb";
pub const GLOBAL_DB_FILE: &str = "global_storage.sqlite";

const PARTIAL_SUFFIX: &str = ".partial";

#[derive(Debug, thiserror::Error)]
pub enum BackupSnapshotError {
    #[error(
        "A backup snapshot already exists at {path}. Copy it off the host if it is still needed, then delete it \
         before taking another."
    )]
    SnapshotAlreadyExists { path: PathBuf },
    #[error("Could not prepare backup directory {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("Could not checkpoint the state store: {0}")]
    StateStore(#[from] StorageError),
    #[error("Could not copy the global database: {0}")]
    GlobalDb(#[from] SqliteStorageError),
}

/// Writes a snapshot into a new directory under `backups_dir` named for `now`, and returns its path.
///
/// The directory only appears under its final name once both copies are complete. A failed attempt removes what it
/// wrote; one interrupted by the process exiting leaves a `*.partial` directory, which blocks the next snapshot
/// until it is deleted.
pub fn create_backup_snapshot(
    state_store: &ValidatorNodeStateStore,
    global_db: &GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>,
    backups_dir: &Path,
    now: OffsetDateTime,
) -> Result<PathBuf, BackupSnapshotError> {
    fs::create_dir_all(backups_dir).map_err(io_error(backups_dir))?;
    if let Some(existing) = fs::read_dir(backups_dir).map_err(io_error(backups_dir))?.next() {
        let existing = existing.map_err(io_error(backups_dir))?;
        return Err(BackupSnapshotError::SnapshotAlreadyExists { path: existing.path() });
    }

    let name = snapshot_name(now);
    let partial = backups_dir.join(format!("{name}{PARTIAL_SUFFIX}"));
    fs::create_dir_all(&partial).map_err(io_error(&partial))?;

    let result = write_snapshot(state_store, global_db, &partial).and_then(|()| {
        let path = backups_dir.join(&name);
        fs::rename(&partial, &path).map_err(io_error(&path))?;
        Ok(path)
    });
    if result.is_err() {
        let _ignore = fs::remove_dir_all(&partial);
    }
    result
}

fn write_snapshot(
    state_store: &ValidatorNodeStateStore,
    global_db: &GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>,
    dir: &Path,
) -> Result<(), BackupSnapshotError> {
    state_store.create_checkpoint(dir.join(STATE_STORE_DIR))?;
    global_db.adapter().copy_into(&dir.join(GLOBAL_DB_FILE))?;
    Ok(())
}

fn snapshot_name(now: OffsetDateTime) -> String {
    let now = now.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> BackupSnapshotError + '_ {
    move |source| BackupSnapshotError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use tari_consensus_types::{BlockId, LastVoted};
    use tari_ootle_common_types::{Epoch, NodeHeight};
    use tari_ootle_storage::{StateStore, StateStoreReadTransaction, StateStoreWriteTransaction, global::DbFactory};
    use tari_ootle_storage_sqlite::SqliteDbFactory;
    use tari_state_store_rocksdb::DatabaseOptions;

    use super::*;

    fn open_global_db(path: &Path) -> GlobalDb<SqliteGlobalDbAdapter<PeerAddress>> {
        let factory = SqliteDbFactory::new(path);
        factory.migrate().unwrap();
        factory.get_or_create_global_db().unwrap()
    }

    fn set_marker(db: &GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>, value: u64) {
        let mut tx = db.create_transaction().unwrap();
        db.metadata(&mut tx).set_metadata(b"marker", &value).unwrap();
        db.commit(tx).unwrap();
    }

    fn get_marker(db: &GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>) -> Option<u64> {
        let mut tx = db.create_transaction().unwrap();
        db.metadata(&mut tx).get_metadata(b"marker").unwrap()
    }

    fn set_last_voted(store: &ValidatorNodeStateStore, height: u64) {
        store
            .with_write_tx(|tx| {
                tx.last_voted_set(&LastVoted {
                    block_id: BlockId::zero(),
                    height: NodeHeight(height),
                    epoch: Epoch::zero(),
                })
            })
            .unwrap();
    }

    fn last_voted_height(store: &ValidatorNodeStateStore) -> NodeHeight {
        store
            .create_read_tx()
            .unwrap()
            .last_voted_get(Epoch::zero())
            .unwrap()
            .height
    }

    fn at(seconds: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(seconds).unwrap()
    }

    struct Node {
        _dir: tempfile::TempDir,
        state_store: ValidatorNodeStateStore,
        global_db: GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>,
        backups_dir: PathBuf,
    }

    impl Node {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let state_store =
                ValidatorNodeStateStore::open(dir.path().join(STATE_STORE_DIR), DatabaseOptions::default()).unwrap();
            let global_db = open_global_db(&dir.path().join(GLOBAL_DB_FILE));
            let backups_dir = dir.path().join("backups");
            Self {
                _dir: dir,
                state_store,
                global_db,
                backups_dir,
            }
        }

        fn snapshot(&self, now: OffsetDateTime) -> Result<PathBuf, BackupSnapshotError> {
            create_backup_snapshot(&self.state_store, &self.global_db, &self.backups_dir, now)
        }
    }

    #[test]
    fn snapshot_holds_both_databases_as_they_were_when_it_was_taken() {
        let node = Node::new();
        set_last_voted(&node.state_store, 1);
        set_marker(&node.global_db, 1);

        let path = node.snapshot(at(1_700_000_000)).unwrap();
        assert_eq!(path, node.backups_dir.join("20231114T221320Z"));
        set_last_voted(&node.state_store, 2);
        set_marker(&node.global_db, 2);

        let restored_store =
            ValidatorNodeStateStore::open(path.join(STATE_STORE_DIR), DatabaseOptions::default()).unwrap();
        assert_eq!(last_voted_height(&restored_store), NodeHeight(1));
        assert_eq!(get_marker(&open_global_db(&path.join(GLOBAL_DB_FILE))), Some(1));
        assert_eq!(last_voted_height(&node.state_store), NodeHeight(2));
    }

    #[test]
    fn a_snapshot_left_behind_blocks_the_next_one() {
        let node = Node::new();
        let first = node.snapshot(at(1_700_000_000)).unwrap();

        let err = node.snapshot(at(1_700_000_060)).unwrap_err();
        assert!(matches!(err, BackupSnapshotError::SnapshotAlreadyExists { ref path } if *path == first));

        fs::remove_dir_all(&first).unwrap();
        node.snapshot(at(1_700_000_060)).unwrap();
    }
}
