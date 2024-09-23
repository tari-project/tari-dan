//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{Epoch, NodeHeight};

use crate::{
    consensus_models::{Block, BlockId, LeafBlock},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

pub struct LastProposed {
    pub height: NodeHeight,
    pub block_id: BlockId,
    pub epoch: Epoch,
}
impl LastProposed {
    pub fn as_leaf_block(&self) -> LeafBlock {
        LeafBlock {
            block_id: self.block_id,
            height: self.height,
            epoch: self.epoch,
        }
    }
}

impl LastProposed {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &TTx) -> Result<Self, StorageError> {
        tx.last_proposed_get()
    }

    pub fn set<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.last_proposed_set(self)
    }

    pub fn unset<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.last_proposed_unset(self)
    }

    pub fn get_block<TTx: StateStoreReadTransaction + ?Sized>(&self, tx: &TTx) -> Result<Block, StorageError> {
        Block::get(tx, &self.block_id)
    }
}

impl std::fmt::Display for LastProposed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LastProposed({}, BlockId({}), {})",
            self.height, self.block_id, self.epoch
        )
    }
}
