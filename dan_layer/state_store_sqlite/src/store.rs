//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt,
    marker::PhantomData,
    sync::{Arc, Mutex},
};

use diesel::{sql_query, Connection, RunQueryDsl, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use serde::{de::DeserializeOwned, Serialize};
use tari_dan_common_types::NodeAddressable;
use tari_dan_storage::{StateStore, StorageError};

use crate::{
    error::SqliteStorageError,
    reader::SqliteStateStoreReadTransaction,
    sqlite_transaction::SqliteTransaction,
    writer::SqliteStateStoreWriteTransaction,
};

const _LOG_TARGET: &str = "tari::dan::storage::sqlite::state_store";

pub struct SqliteStateStore<TAddr> {
    connection: Arc<Mutex<SqliteConnection>>,
    _addr: PhantomData<TAddr>,
}

impl<TAddr> SqliteStateStore<TAddr> {
    pub fn connect(url: &str) -> Result<Self, StorageError> {
        let mut connection = SqliteConnection::establish(url).map_err(SqliteStorageError::from)?;

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
            _addr: PhantomData,
        })
    }
}

// Manually implement the Debug implementation because `SqliteConnection` does not implement the Debug trait
impl<TAddr> fmt::Debug for SqliteStateStore<TAddr> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SqliteShardStore")
    }
}

impl<TAddr: NodeAddressable + Serialize + DeserializeOwned> StateStore for SqliteStateStore<TAddr> {
    type Addr = TAddr;
    type ReadTransaction<'a> = SqliteStateStoreReadTransaction<'a, Self::Addr> where TAddr: 'a;
    type WriteTransaction<'a> = SqliteStateStoreWriteTransaction<'a, Self::Addr> where TAddr: 'a;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, StorageError> {
        let tx = SqliteTransaction::begin(self.connection.lock().unwrap())?;
        Ok(SqliteStateStoreReadTransaction::new(tx))
    }

    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, StorageError> {
        let tx = SqliteTransaction::begin(self.connection.lock().unwrap())?;
        Ok(SqliteStateStoreWriteTransaction::new(tx))
    }
}

impl<TAddr> Clone for SqliteStateStore<TAddr> {
    fn clone(&self) -> Self {
        Self {
            connection: self.connection.clone(),
            _addr: PhantomData,
        }
    }
}
