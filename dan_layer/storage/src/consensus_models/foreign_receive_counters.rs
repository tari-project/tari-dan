//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use tari_dan_common_types::{optional::Optional, shard::Shard};

use crate::{StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

#[derive(Debug, Clone)]
pub struct ForeignReceiveCounters {
    pub counters: HashMap<Shard, u64>,
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

    pub fn increment(&mut self, bucket: &Shard) {
        *self.counters.entry(*bucket).or_default() += 1;
    }

    /// Returns the counter for the provided shard. If the count does not exist, 0 is returned.
    pub fn get_count(&self, bucket: &Shard) -> u64 {
        self.counters.get(bucket).copied().unwrap_or_default()
    }
}

impl ForeignReceiveCounters {
    pub fn save<TTx: StateStoreWriteTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.foreign_receive_counters_set(self)?;
        Ok(())
    }

    pub fn get_or_default<TTx: StateStoreReadTransaction + ?Sized>(tx: &mut TTx) -> Result<Self, StorageError> {
        Ok(tx.foreign_receive_counters_get().optional()?.unwrap_or_default())
    }
}
