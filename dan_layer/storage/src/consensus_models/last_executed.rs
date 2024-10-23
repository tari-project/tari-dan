//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{Epoch, NodeHeight};

use crate::{consensus_models::BlockId, StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

pub struct LastExecuted {
    pub height: NodeHeight,
    pub block_id: BlockId,
    pub epoch: Epoch,
}

impl LastExecuted {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &TTx) -> Result<Self, StorageError> {
        tx.last_executed_get()
    }

    pub fn set<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.last_executed_set(self)
    }
}
