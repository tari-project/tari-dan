//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use tari_dan_common_types::{optional::Optional, shard::Shard};

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

    pub fn increment_counter(&mut self, shard: Shard) -> u64 {
        *self.counters.entry(shard).and_modify(|v| *v += 1).or_insert(1)
    }

    pub fn get_count(&self, shard: Shard) -> u64 {
        self.counters.get(&shard).copied().unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.counters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.counters.is_empty()
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

    pub fn get_or_default<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        block_id: &BlockId,
    ) -> Result<Self, StorageError> {
        Ok(tx.foreign_send_counters_get(block_id).optional()?.unwrap_or_default())
    }
}
