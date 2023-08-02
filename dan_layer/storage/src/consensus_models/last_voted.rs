//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{Epoch, NodeHeight};

use crate::{consensus_models::BlockId, StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

pub struct LastVoted {
    pub block_id: BlockId,
    pub height: NodeHeight,
}

impl LastVoted {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx) -> Result<Self, StorageError> {
        tx.last_voted_get()
    }

    pub fn set<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.last_voted_set(self)
    }
}
