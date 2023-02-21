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

use std::sync::MutexGuard;

use diesel::{sql_query, RunQueryDsl, SqliteConnection};

use crate::error::SqliteStorageError;

const _LOG_TARGET: &str = "tari::dan::storage::sqlite::transaction";

pub struct SqliteTransaction<'a> {
    connection: MutexGuard<'a, SqliteConnection>,
    is_done: bool,
}

impl<'a> SqliteTransaction<'a> {
    pub fn begin(connection: MutexGuard<'a, SqliteConnection>) -> Result<Self, SqliteStorageError> {
        let mut this = Self {
            connection,
            is_done: false,
        };
        this.execute_sql("BEGIN TRANSACTION")?;
        Ok(this)
    }

    pub fn connection(&mut self) -> &mut SqliteConnection {
        &mut self.connection
    }

    pub fn commit(mut self) -> Result<(), SqliteStorageError> {
        self.execute_sql("COMMIT")?;
        self.is_done = true;
        Ok(())
    }

    pub fn rollback(mut self) -> Result<(), SqliteStorageError> {
        self.rollback_inner()
    }

    pub fn execute_sql(&mut self, sql: &str) -> Result<(), SqliteStorageError> {
        sql_query(sql)
            .execute(self.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "execute sql".to_string(),
            })?;

        Ok(())
    }

    fn rollback_inner(&mut self) -> Result<(), SqliteStorageError> {
        self.execute_sql("ROLLBACK")?;
        self.is_done = true;
        Ok(())
    }
}

impl Drop for SqliteTransaction<'_> {
    fn drop(&mut self) {
        if !self.is_done {
            let _ignore = self.rollback_inner();
        }
    }
}
