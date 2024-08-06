//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use indexmap::IndexMap;
use tari_dan_common_types::{shard::Shard, Epoch};
use tari_state_tree::{Hash, SPARSE_MERKLE_PLACEHOLDER_HASH};

use crate::{
    consensus_models::{Block, QuorumCertificate},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone)]
pub struct EpochCheckpoint {
    block: Block,
    linked_qcs: Vec<QuorumCertificate>,
    shard_roots: IndexMap<Shard, Hash>,
}

impl EpochCheckpoint {
    pub fn new(block: Block, linked_qcs: Vec<QuorumCertificate>, shard_roots: IndexMap<Shard, Hash>) -> Self {
        Self {
            block,
            linked_qcs,
            shard_roots,
        }
    }

    pub fn qcs(&self) -> &[QuorumCertificate] {
        &self.linked_qcs
    }

    pub fn block(&self) -> &Block {
        &self.block
    }

    pub fn shard_roots(&self) -> &IndexMap<Shard, Hash> {
        &self.shard_roots
    }

    pub fn get_shard_root(&self, shard: Shard) -> Hash {
        self.shard_roots
            .get(&shard)
            .copied()
            .unwrap_or(SPARSE_MERKLE_PLACEHOLDER_HASH)
    }
}

impl EpochCheckpoint {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &TTx, epoch: Epoch) -> Result<Self, StorageError> {
        tx.epoch_checkpoint_get(epoch)
    }

    pub fn save<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.epoch_checkpoint_save(self)
    }
}

impl Display for EpochCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EpochCheckpoint: block={}, qcs=", self.block)?;
        for qc in self.qcs() {
            write!(f, "{}, ", qc.id())?;
        }
        Ok(())
    }
}
