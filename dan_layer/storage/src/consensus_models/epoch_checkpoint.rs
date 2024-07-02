//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use tari_dan_common_types::Epoch;

use crate::{
    consensus_models::{Block, QuorumCertificate},
    StateStoreReadTransaction,
    StorageError,
};

#[derive(Debug, Clone)]
pub struct EpochCheckpoint {
    block: Block,
    qcs: Vec<QuorumCertificate>,
}

impl EpochCheckpoint {
    pub fn new(block: Block, qcs: Vec<QuorumCertificate>) -> Self {
        Self { block, qcs }
    }

    pub fn qcs(&self) -> &[QuorumCertificate] {
        &self.qcs
    }

    pub fn block(&self) -> &Block {
        &self.block
    }
}

impl EpochCheckpoint {
    pub fn generate<TTx: StateStoreReadTransaction>(tx: &TTx, epoch: Epoch) -> Result<Self, StorageError> {
        let mut blocks = tx.blocks_get_last_n_in_epoch(3, epoch)?;
        if blocks.is_empty() {
            return Err(StorageError::NotFound {
                item: format!("EpochCheckpoint: No blocks found for epoch {}", epoch),
                key: epoch.to_string(),
            });
        }

        let commit_block = blocks.pop().unwrap();
        let qcs = blocks.into_iter().map(|b| b.into_justify()).collect();

        Ok(Self {
            block: commit_block,
            qcs,
        })
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
