//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::NodeHeight;

use crate::{consensus_models::BlockId, StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

pub struct LastVoted {
    pub block_id: BlockId,
    pub height: NodeHeight,
}

impl LastVoted {
    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }
}

impl LastVoted {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx) -> Result<Self, StorageError> {
        tx.last_voted_get()
    }

    pub fn set<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.last_voted_set(self)
    }

    pub fn unset<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.last_votes_unset(self)
    }
}

impl std::fmt::Display for LastVoted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(block_id: {}, height: {})", self.block_id, self.height)
    }
}
