//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use tari_dan_common_types::shard::Shard;

use super::BlockId;
use crate::{StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

#[derive(Debug, Clone)]
pub struct ForeignSendCounters {
    pub counters: HashMap<Shard, u64>,
}

impl Default for ForeignSendCounters {
    fn default() -> Self {
        Self::new()
    }
}

impl ForeignSendCounters {
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
        }
    }

    pub fn increment_counter(&mut self, bucket: Shard) -> u64 {
        *self.counters.entry(bucket).and_modify(|v| *v += 1).or_insert(1)
    }
}

impl ForeignSendCounters {
    pub fn set<TTx: StateStoreWriteTransaction + ?Sized>(
        &self,
        tx: &mut TTx,
        block_id: &BlockId,
    ) -> Result<(), StorageError> {
        tx.foreign_send_counters_set(self, block_id)?;
        Ok(())
    }

    pub fn get<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &mut TTx,
        block_id: &BlockId,
    ) -> Result<Self, StorageError> {
        tx.foreign_send_counters_get(block_id)
    }
}
