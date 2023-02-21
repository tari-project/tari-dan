// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause
#[macro_use]
extern crate diesel;

mod models;
mod reader;
mod schema;
mod serialization;
mod writer;

use std::{
    fs::create_dir_all,
    path::Path,
    sync::{Arc, Mutex},
};

use diesel::{sql_query, Connection, RunQueryDsl, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use tari_dan_wallet_sdk::storage::{WalletStorageError, WalletStore};

use crate::{reader::ReadTransaction, writer::WriteTransaction};

#[derive(Clone)]
pub struct SqliteWalletStore {
    // MUTEX: required to make Sync
    connection: Arc<Mutex<SqliteConnection>>,
}

impl SqliteWalletStore {
    pub fn try_open<P: AsRef<Path>>(path: P) -> Result<Self, WalletStorageError> {
        create_dir_all(path.as_ref().parent().unwrap()).expect("Failed to create DB path");

        let database_url = path.as_ref().to_str().expect("database_url utf-8 error").to_string();
        let mut connection =
            SqliteConnection::establish(&database_url).map_err(|e| WalletStorageError::general("connect", e))?;

        sql_query("PRAGMA foreign_keys = ON;")
            .execute(&mut connection)
            .map_err(|source| WalletStorageError::general("set pragma", source))?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn run_migrations(&self) -> Result<(), WalletStorageError> {
        let mut conn = self.connection.lock().unwrap();
        const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations");
        conn.run_pending_migrations(MIGRATIONS)
            .map_err(|source| WalletStorageError::general("migrate", source))?;
        Ok(())
    }
}

impl WalletStore for SqliteWalletStore {
    type ReadTransaction<'a> = ReadTransaction<'a>;
    type WriteTransaction<'a> = WriteTransaction<'a>;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, WalletStorageError> {
        let mut lock = self.connection.lock().unwrap();
        sql_query("BEGIN")
            .execute(&mut *lock)
            .map_err(|e| WalletStorageError::general("BEGIN transaction", e))?;
        Ok(ReadTransaction::new(lock))
    }

    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, WalletStorageError> {
        let mut lock = self.connection.lock().unwrap();
        sql_query("BEGIN")
            .execute(&mut *lock)
            .map_err(|e| WalletStorageError::general("BEGIN transaction", e))?;
        Ok(WriteTransaction::new(lock))
    }
}
