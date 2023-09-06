//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::NodeHeight;

use crate::{
    consensus_models::{Block, BlockId},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone)]
pub struct LockedBlock {
    pub height: NodeHeight,
    pub block_id: BlockId,
}

impl LockedBlock {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx) -> Result<Self, StorageError> {
        tx.locked_block_get()
    }

    pub fn get_block<TTx: StateStoreReadTransaction>(&self, tx: &mut TTx) -> Result<Block<TTx::Addr>, StorageError> {
        tx.blocks_get(&self.block_id)
    }

    pub fn set<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.locked_block_set(self)
    }
}
