//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

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
    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }
}

impl LockedBlock {
    pub fn get<TTx: StateStoreReadTransaction + ?Sized>(tx: &TTx) -> Result<Self, StorageError> {
        tx.locked_block_get()
    }

    pub fn get_block<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<Block, StorageError> {
        tx.blocks_get(&self.block_id)
    }

    pub fn set<TTx: StateStoreWriteTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.locked_block_set(self)
    }
}

impl Display for LockedBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LockedBlock({}, {})", self.height, self.block_id)
    }
}
