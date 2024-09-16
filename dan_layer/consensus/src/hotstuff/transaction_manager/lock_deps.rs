//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use indexmap::IndexMap;
use tari_dan_storage::{
    consensus_models::{BlockId, LockConflict},
    StateStoreWriteTransaction,
    StorageError,
};
use tari_transaction::TransactionId;

pub struct TransactionLockConflicts {
    conflicts: IndexMap<TransactionId, Vec<LockConflict>>,
}

impl TransactionLockConflicts {
    pub fn new() -> Self {
        Self {
            conflicts: IndexMap::new(),
        }
    }

    pub fn add(&mut self, transaction_id: TransactionId, conflicts: Vec<LockConflict>) {
        self.conflicts.insert(transaction_id, conflicts);
    }
}

impl TransactionLockConflicts {
    pub(crate) fn save_for_block<TTx: StateStoreWriteTransaction>(
        &self,
        tx: &mut TTx,
        block_id: &BlockId,
    ) -> Result<(), StorageError> {
        if self.conflicts.is_empty() {
            return Ok(());
        }

        tx.lock_conflicts_insert_all(block_id, &self.conflicts)
    }
}
