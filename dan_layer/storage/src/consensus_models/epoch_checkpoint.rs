//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::FixedHash;
use tari_dan_common_types::{shard::Shard, Epoch, PeerAddress};

use crate::{
    consensus_models::{Block, BlockId, QcId, QuorumCertificate},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

pub struct EpochCheckpoint {
    epoch: Epoch,
    shard: Shard,
    block_id: BlockId,
    state_root: FixedHash,
    qcs: [QcId; 3],
}

impl EpochCheckpoint {
    pub fn new(block_id: BlockId, epoch: Epoch, shard: Shard, state_root: FixedHash, qcs: [QcId; 3]) -> Self {
        Self {
            block_id,
            epoch,
            shard,
            state_root,
            qcs,
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn shard(&self) -> Shard {
        self.shard
    }

    pub fn state_root(&self) -> &FixedHash {
        &self.state_root
    }

    pub fn qcs(&self) -> &[QcId; 3] {
        &self.qcs
    }

    pub fn block_id(&self) -> BlockId {
        self.block_id
    }
}

impl EpochCheckpoint {
    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.epoch_checkpoint_insert(self)
    }

    pub fn get_for_epoch<TTx: StateStoreReadTransaction>(tx: &TTx, epoch: Epoch) -> Result<Self, StorageError> {
        tx.epoch_checkpoints_get_by_epoch(epoch)
    }

    pub fn get_block<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<Block, StorageError> {
        tx.block_get(self.block_id)
    }

    pub fn get_qcs<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<Vec<QuorumCertificate>, StorageError> {
        tx.quorum_certificates_get_all(self.qcs)
    }
}
