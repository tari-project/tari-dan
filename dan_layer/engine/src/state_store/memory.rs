//  Copyright 2022. The Tari Project
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
use std::{
    collections::HashMap,
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use anyhow::anyhow;
use tari_bor::encode;
use tari_dan_common_types::SubstateState;
use tari_utilities::hex::to_hex;

use crate::state_store::{AtomicDb, StateReader, StateStoreError, StateWriter};

type InnerKvMap = HashMap<Vec<u8>, Vec<u8>>;

#[derive(Debug, Clone)]
pub struct MemoryStateStore {
    pub allow_creation_of_non_existent_shards: bool,
    state: Arc<RwLock<InnerKvMap>>,
}

impl Default for MemoryStateStore {
    fn default() -> Self {
        Self {
            allow_creation_of_non_existent_shards: true,
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl MemoryStateStore {
    pub fn load<K: Into<Vec<u8>>>(values: Vec<(K, SubstateState)>) -> Self {
        let mut state = HashMap::new();
        for (k, v) in values {
            state.insert(k.into(), encode(&v).expect("Could not encode substate"));
        }
        Self {
            allow_creation_of_non_existent_shards: false,
            state: Arc::new(RwLock::new(state)),
        }
    }
}

pub struct MemoryTransaction<T> {
    pending: InnerKvMap,
    // TODO: this is copied from the state store, there's probably a better way
    allow_creation_of_non_existent_shards: bool,
    guard: T,
}

impl<'a> AtomicDb<'a> for MemoryStateStore {
    type Error = anyhow::Error;
    type ReadAccess = MemoryTransaction<RwLockReadGuard<'a, InnerKvMap>>;
    type WriteAccess = MemoryTransaction<RwLockWriteGuard<'a, InnerKvMap>>;

    fn read_access(&'a self) -> Result<Self::ReadAccess, Self::Error> {
        let guard = self.state.read().map_err(|_| anyhow!("Failed to read state"))?;

        Ok(MemoryTransaction {
            pending: HashMap::default(),
            allow_creation_of_non_existent_shards: self.allow_creation_of_non_existent_shards,
            guard,
        })
    }

    fn write_access(&'a self) -> Result<Self::WriteAccess, Self::Error> {
        let guard = self.state.write().map_err(|_| anyhow!("Failed to write state"))?;

        Ok(MemoryTransaction {
            pending: HashMap::default(),
            allow_creation_of_non_existent_shards: self.allow_creation_of_non_existent_shards,
            guard,
        })
    }
}

impl<'a> StateReader for MemoryTransaction<RwLockReadGuard<'a, InnerKvMap>> {
    fn get_state_raw(&self, key: &[u8]) -> Result<Vec<u8>, StateStoreError> {
        self.pending
            .get(key)
            .cloned()
            .or_else(|| self.guard.get(key).cloned())
            .ok_or_else(|| StateStoreError::NotFound {
                kind: "state",
                key: to_hex(key),
            })
    }

    fn exists(&self, key: &[u8]) -> Result<bool, StateStoreError> {
        Ok(self.pending.contains_key(key) || self.guard.contains_key(key))
    }
}

impl<'a> StateReader for MemoryTransaction<RwLockWriteGuard<'a, InnerKvMap>> {
    fn get_state_raw(&self, key: &[u8]) -> Result<Vec<u8>, StateStoreError> {
        self.pending
            .get(key)
            .cloned()
            .or_else(|| self.guard.get(key).cloned())
            .ok_or_else(|| StateStoreError::NotFound {
                kind: "state",
                key: to_hex(key),
            })
    }

    fn exists(&self, key: &[u8]) -> Result<bool, StateStoreError> {
        Ok(self.pending.contains_key(key) || self.guard.contains_key(key))
    }
}

impl<'a> StateWriter for MemoryTransaction<RwLockWriteGuard<'a, InnerKvMap>> {
    fn set_state_raw(&mut self, key: &[u8], value: Vec<u8>) -> Result<(), StateStoreError> {
        self.pending.insert(key.to_vec(), value);
        Ok(())
    }

    fn delete_state_raw(&mut self, key: &[u8]) -> Result<(), StateStoreError> {
        self.pending.remove(key);
        self.guard.remove(key);
        Ok(())
    }

    fn commit(mut self) -> Result<(), StateStoreError> {
        if self.allow_creation_of_non_existent_shards {
            self.guard.extend(self.pending);
        } else {
            for (k, v) in self.pending {
                if let Some(val) = self.guard.get_mut(&k) {
                    *val = v;
                } else {
                    return Err(StateStoreError::NonExistentShard { shard: k });
                }
            }
        }
        // self.guard.extend(self.pending.into_iter());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use tari_dan_common_types::optional::Optional;

    use super::*;

    #[test]
    fn read_write() {
        let store = MemoryStateStore::default();
        let mut access = store.write_access().unwrap();
        access.set_state_raw(b"abc", vec![1, 2, 3]).unwrap();
        let res = access.get_state_raw(b"abc").unwrap();
        assert_eq!(res, vec![1, 2, 3]);
        let res = access.get_state_raw(b"def").optional().unwrap();
        assert_eq!(res, None);
    }

    #[test]
    fn read_write_rollback_commit() {
        #[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
        struct UserData {
            name: String,
            age: u8,
        }

        let user_data = UserData {
            name: "Foo".to_string(),
            age: 99,
        };

        let store = MemoryStateStore::default();
        {
            let mut access = store.write_access().unwrap();
            access.set_state(b"abc", user_data.clone()).unwrap();
            let res: UserData = access.get_state(b"abc").unwrap();
            assert_eq!(res, user_data);
            let res = access.get_state::<_, UserData>(b"def").optional().unwrap();
            assert_eq!(res, None);
            // Drop without commit rolls back
        }

        {
            let access = store.read_access().unwrap();
            let res = access.get_state::<_, UserData>(b"abc").optional().unwrap();
            assert_eq!(res, None);
        }

        {
            let mut access = store.write_access().unwrap();
            access.set_state(b"abc", user_data.clone()).unwrap();
            access.commit().unwrap();
        }

        let access = store.read_access().unwrap();
        let res: UserData = access.get_state(b"abc").unwrap();
        assert_eq!(res, user_data);
    }
}
