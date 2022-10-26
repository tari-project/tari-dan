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

use diesel::{Connection, SqliteConnection};

use crate::error::SqliteStorageError;

const LOG_TARGET: &str = "storage::sqlite::transaction";

pub struct SqliteTransaction {
    connection: SqliteConnection,
    is_done: bool,
}

impl SqliteTransaction {
    pub fn begin(connection: SqliteConnection) -> Result<Self, SqliteStorageError> {
        // TODO: This busy wait sucks and there is definitely a better way, but we care more about the SQLite DB working
        //       than performance
        while let Err(err) = connection.execute("BEGIN EXCLUSIVE;") {
            if err.to_string().contains("database is locked") {
                log::warn!(target: LOG_TARGET, "Database is locked, retrying in 100ms");
                std::thread::sleep(std::time::Duration::from_millis(100));
            } else {
                return Err(SqliteStorageError::DieselError {
                    source: err,
                    operation: "begin transaction".to_string(),
                });
            }
        }

        Ok(Self {
            connection,
            is_done: false,
        })
    }

    pub fn connection(&self) -> &SqliteConnection {
        &self.connection
    }

    pub fn commit(mut self) -> Result<(), SqliteStorageError> {
        self.connection
            .execute("COMMIT")
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "commit".to_string(),
            })?;

        self.is_done = true;
        Ok(())
    }

    pub fn rollback(mut self) -> Result<(), SqliteStorageError> {
        self.rollback_inner()
    }

    fn rollback_inner(&mut self) -> Result<(), SqliteStorageError> {
        self.connection
            .execute("ROLLBACK")
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "commit".to_string(),
            })?;

        self.is_done = true;
        Ok(())
    }
}

impl Drop for SqliteTransaction {
    fn drop(&mut self) {
        if !self.is_done {
            log::warn!(
                target: LOG_TARGET,
                "SqliteTransaction was dropped without being committed or rolled back"
            );
            let _ignore = self.rollback_inner();
        }
    }
}
