//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt,
    fs::create_dir_all,
    path::Path,
    sync::{Arc, Mutex},
};

use diesel::{sql_query, Connection, RunQueryDsl, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use tari_dan_storage::{StateStore, StorageError};

use crate::{
    error::SqliteStorageError,
    reader::SqliteStateStoreReadTransaction,
    sqlite_transaction::SqliteTransaction,
    writer::SqliteStateStoreWriteTransaction,
};

const _LOG_TARGET: &str = "tari::dan::storage::sqlite::state_store";

#[derive(Clone)]
pub struct SqliteStateStore {
    connection: Arc<Mutex<SqliteConnection>>,
}

impl SqliteStateStore {
    pub fn connect<P: AsRef<Path>>(path: P) -> Result<Self, StorageError> {
        create_dir_all(path.as_ref().parent().unwrap()).map_err(|_| StorageError::FileSystemPathDoesNotExist)?;

        let database_url = path.as_ref().to_str().expect("database_url utf-8 error").to_string();
        let mut connection = SqliteConnection::establish(&database_url).map_err(SqliteStorageError::from)?;

        const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations");
        connection
            .run_pending_migrations(MIGRATIONS)
            .map_err(|source| SqliteStorageError::MigrationError { source })?;

        sql_query("PRAGMA foreign_keys = ON;")
            .execute(&mut connection)
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "set pragma",
            })?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }
}

// we mock the Debug implementation because "SqliteConnection" does not implement the Debug trait
impl fmt::Debug for SqliteStateStore {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SqliteShardStore")
    }
}

impl StateStore for SqliteStateStore {
    type ReadTransaction<'a> = SqliteStateStoreReadTransaction<'a>;
    type WriteTransaction<'a> = SqliteStateStoreWriteTransaction<'a>;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, StorageError> {
        let tx = SqliteTransaction::begin(self.connection.lock().unwrap())?;
        Ok(SqliteStateStoreReadTransaction::new(tx))
    }

    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, StorageError> {
        let tx = SqliteTransaction::begin(self.connection.lock().unwrap())?;
        Ok(SqliteStateStoreWriteTransaction::new(tx))
    }
}
