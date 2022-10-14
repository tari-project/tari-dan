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

use diesel::{Connection, ConnectionError, SqliteConnection};
use diesel_migrations::embed_migrations;
use tari_common_types::types::FixedHash;
use tari_dan_core::storage::{DbFactory, StorageError};
use tari_dan_engine::state::StateDb;
use tari_dan_storage::global::GlobalDb;
use tari_utilities::hex::Hex;

use crate::{error::SqliteStorageError, global::SqliteGlobalDbAdapter};

#[derive(Clone)]
pub struct SqliteDbFactory {
    data_dir: PathBuf,
}

impl SqliteDbFactory {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    fn database_url_for(&self, contract_id: &FixedHash) -> String {
        self.data_dir
            .join("asset_data")
            .join(contract_id.to_hex())
            .join("dan_storage.sqlite")
            .into_os_string()
            .into_string()
            .expect("Should not fail")
    }

    fn try_connect(&self, url: &str) -> Result<Option<SqliteConnection>, StorageError> {
        match SqliteConnection::establish(url) {
            Ok(connection) => {
                connection
                    .execute("PRAGMA foreign_keys = ON;")
                    .map_err(|source| SqliteStorageError::DieselError {
                        source,
                        operation: "set pragma".to_string(),
                    })?;
                Ok(Some(connection))
            },
            Err(ConnectionError::BadConnection(_)) => Ok(None),
            Err(err) => Err(SqliteStorageError::from(err).into()),
        }
    }
}

impl DbFactory for SqliteDbFactory {
    type GlobalDbAdapter = SqliteGlobalDbAdapter;

    fn get_or_create_global_db(&self) -> Result<GlobalDb<Self::GlobalDbAdapter>, StorageError> {
        let database_url = self
            .data_dir
            .join("global_storage.sqlite")
            .into_os_string()
            .into_string()
            .expect("Should not fail");

        create_dir_all(&PathBuf::from(&database_url).parent().unwrap())
            .map_err(|_| StorageError::FileSystemPathDoesNotExist)?;

        let connection = SqliteConnection::establish(database_url.as_str()).map_err(SqliteStorageError::from)?;
        connection
            .execute("PRAGMA foreign_keys = ON;")
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "set pragma".to_string(),
            })?;
        embed_migrations!("./global_db_migrations");
        // embedded_migrations::run(&connection).map_err(SqliteStorageError::from)?;
        embedded_migrations::run_with_output(&connection, &mut std::io::stdout()).expect("Migration failed");
        Ok(GlobalDb::new(SqliteGlobalDbAdapter::new(database_url)))
    }
}
