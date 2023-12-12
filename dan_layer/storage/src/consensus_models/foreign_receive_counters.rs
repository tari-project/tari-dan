//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use tari_dan_common_types::shard_bucket::ShardBucket;

use crate::{StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

#[derive(Debug, Clone)]
pub struct ForeignReceiveCounters {
    pub counters: HashMap<ShardBucket, u64>,
}

impl Default for ForeignReceiveCounters {
    fn default() -> Self {
        Self::new()
    }
}

impl ForeignReceiveCounters {
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
        }
    }

    pub fn increment(&mut self, bucket: &ShardBucket) {
        *self.counters.entry(*bucket).or_default() += 1;
    }

    // If we haven't received any messages from this shard yet, return 0
    pub fn get_index(&self, bucket: &ShardBucket) -> u64 {
        self.counters.get(bucket).copied().unwrap_or_default()
    }
}

impl ForeignReceiveCounters {
    pub fn save<TTx: StateStoreWriteTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.foreign_receive_counters_set(self)?;
        Ok(())
    }

    pub fn get<TTx: StateStoreReadTransaction + ?Sized>(tx: &mut TTx) -> Result<Self, StorageError> {
        tx.foreign_receive_counters_get()
    }
}
