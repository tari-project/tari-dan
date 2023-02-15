// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause
#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_migrations;

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

use diesel::{Connection, SqliteConnection};
use diesel_migrations::embed_migrations;
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
        let connection =
            SqliteConnection::establish(&database_url).map_err(|e| WalletStorageError::general("connect", e))?;

        connection
            .execute("PRAGMA foreign_keys = ON;")
            .map_err(|source| WalletStorageError::general("set pragma", source))?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn run_migrations(&self) -> Result<(), WalletStorageError> {
        self.run_migrations_with_output(&mut std::io::sink())
    }

    pub fn run_migrations_with_output<W: std::io::Write>(&self, output: &mut W) -> Result<(), WalletStorageError> {
        let conn = self.connection.lock().unwrap();
        embed_migrations!("./migrations");
        embedded_migrations::run_with_output(&*conn, output).map_err(|e| WalletStorageError::general("migrate", e))?;
        Ok(())
    }
}

impl WalletStore for SqliteWalletStore {
    type ReadTransaction<'a> = ReadTransaction<'a>;
    type WriteTransaction<'a> = WriteTransaction<'a>;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, WalletStorageError> {
        let lock = self.connection.lock().unwrap();
        lock.execute("BEGIN")
            .map_err(|e| WalletStorageError::general("BEGIN transaction", e))?;
        Ok(ReadTransaction::new(lock))
    }

    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, WalletStorageError> {
        let lock = self.connection.lock().unwrap();
        lock.execute("BEGIN")
            .map_err(|e| WalletStorageError::general("BEGIN transaction", e))?;
        Ok(WriteTransaction::new(lock))
    }
}
