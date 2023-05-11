//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::sync::Arc;

use tari_common_types::types::PublicKey;
use tari_dan_core::{
    models::TariDanPayload,
    storage::{shard_store::ShardStore, StorageError},
};
use tari_dan_storage_sqlite::sqlite_shard_store_factory::{
    SqliteShardStore,
    SqliteShardStoreReadTransaction,
    SqliteShardStoreWriteTransaction,
};
use tempdir::TempDir;

#[derive(Clone)]
pub struct TempShardStoreFactory {
    sqlite: SqliteShardStore,
    path: Arc<TempDir>,
}

impl TempShardStoreFactory {
    pub fn new() -> Self {
        let temp_path = TempDir::new("tempshardstore").unwrap();
        let sqlite = SqliteShardStore::try_create(temp_path.path().join("state.db")).unwrap();
        Self {
            sqlite,
            path: Arc::new(temp_path),
        }
    }

    pub fn path(&self) -> &std::path::Path {
        self.path.path()
    }
}

impl Default for TempShardStoreFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ShardStore for TempShardStoreFactory {
    type Addr = PublicKey;
    type Payload = TariDanPayload;
    type ReadTransaction<'a> = SqliteShardStoreReadTransaction<'a>;
    type WriteTransaction<'a> = SqliteShardStoreWriteTransaction<'a>;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, StorageError> {
        self.sqlite.create_read_tx()
    }

    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, StorageError> {
        self.sqlite.create_write_tx()
    }
}

// impl Drop for TempShardStoreFactory {
//     fn drop(&mut self) {
//         if self.delete_on_drop && Path::new(&self.path).exists() {
//             fs::remove_dir_all(&self.path).expect("Could not delete temporary file");
//         }
//     }
// }
