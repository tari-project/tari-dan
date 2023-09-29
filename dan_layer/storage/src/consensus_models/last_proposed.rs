//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::NodeHeight;

use crate::{
    consensus_models::{Block, BlockId},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

pub struct LastProposed {
    pub height: NodeHeight,
    pub block_id: BlockId,
}

impl LastProposed {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx) -> Result<Self, StorageError> {
        tx.last_proposed_get()
    }

    pub fn set<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.last_proposed_set(self)
    }

    pub fn unset<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.last_proposed_unset(self)
    }

    pub fn get_block<TTx: StateStoreReadTransaction + ?Sized>(
        &self,
        tx: &mut TTx,
    ) -> Result<Block<TTx::Addr>, StorageError> {
        Block::get(tx, &self.block_id)
    }
}
