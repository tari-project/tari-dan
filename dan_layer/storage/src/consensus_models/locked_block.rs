//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{Epoch, NodeHeight};

use crate::{
    consensus_models::{Block, BlockId, QuorumCertificate},
    StateStoreReadTransaction, StateStoreWriteTransaction, StorageError,
};

#[derive(Debug, Clone)]
pub struct LockedBlock {
    pub epoch: Epoch,
    pub height: NodeHeight,
    pub block_id: BlockId,
}

impl LockedBlock {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx, epoch: Epoch) -> Result<Self, StorageError> {
        tx.locked_block_get(epoch)
    }

    pub fn get_block<TTx: StateStoreReadTransaction>(&self, tx: &mut TTx) -> Result<Block, StorageError> {
        tx.blocks_get(&self.block_id)
    }

    pub fn get_quorum_certificate<TTx: StateStoreReadTransaction>(
        &self,
        tx: &mut TTx,
    ) -> Result<QuorumCertificate, StorageError> {
        tx.quorum_certificates_get(&self.block_id)
    }

    pub fn set<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.locked_block_set(self)
    }
}
