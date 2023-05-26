//  Copyright 2021. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{fs::create_dir_all, path::PathBuf};

use diesel::{sql_query, Connection, RunQueryDsl, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use tari_dan_storage::{
    global::{DbFactory, GlobalDb},
    StorageError,
};

use crate::{error::SqliteStorageError, global::SqliteGlobalDbAdapter};

const _LOG_TARGET: &str = "tari::dan::sqlite_storage";

#[derive(Debug, Clone)]
pub struct SqliteDbFactory {
    data_dir: PathBuf,
}

impl SqliteDbFactory {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    fn connect(&self) -> Result<SqliteConnection, StorageError> {
        let database_url = self.data_dir.join("global_storage.sqlite");
        create_dir_all(database_url.parent().unwrap()).map_err(|_| StorageError::FileSystemPathDoesNotExist)?;
        let database_url = database_url.to_str().expect("database_url utf-8 error").to_string();
        let connection = SqliteConnection::establish(&database_url).map_err(SqliteStorageError::from)?;
        Ok(connection)
    }
}

impl DbFactory for SqliteDbFactory {
    type GlobalDbAdapter = SqliteGlobalDbAdapter;

    fn get_or_create_global_db(&self) -> Result<GlobalDb<Self::GlobalDbAdapter>, StorageError> {
        let mut connection = self.connect()?;
        sql_query("PRAGMA foreign_keys = ON;")
            .execute(&mut connection)
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "set pragma".to_string(),
            })?;
        Ok(GlobalDb::new(SqliteGlobalDbAdapter::new(connection)))
    }

    fn migrate(&self) -> Result<(), StorageError> {
        let mut connection = self.connect()?;
        const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./global_db_migrations");
        connection
            .run_pending_migrations(MIGRATIONS)
            .map_err(|source| SqliteStorageError::MigrationError { source })?;

        Ok(())
    }
}
