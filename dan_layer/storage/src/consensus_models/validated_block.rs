//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, fmt::Display, ops::DerefMut};

use tari_common_types::types::PublicKey;
use tari_dan_common_types::{Epoch, NodeHeight};

use crate::{
    consensus_models::{Block, BlockId},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

pub struct ValidBlock {
    block: Block,
    dummy_blocks: Vec<Block>,
}

impl ValidBlock {
    pub fn new(block: Block) -> Self {
        Self {
            block,
            dummy_blocks: vec![],
        }
    }

    pub fn with_dummy_blocks(block: Block, dummy_blocks: Vec<Block>) -> Self {
        Self { block, dummy_blocks }
    }

    pub fn block(&self) -> &Block {
        &self.block
    }

    pub fn id(&self) -> &BlockId {
        self.block.id()
    }

    pub fn height(&self) -> NodeHeight {
        self.block.height()
    }

    pub fn epoch(&self) -> Epoch {
        self.block.epoch()
    }

    pub fn proposed_by(&self) -> &PublicKey {
        self.block.proposed_by()
    }

    pub fn dummy_blocks(&self) -> &[Block] {
        &self.dummy_blocks
    }
}

impl ValidBlock {
    pub fn save_all_dummy_blocks<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction + DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        for block in &self.dummy_blocks {
            block.save(tx)?;
        }
        Ok(())
    }
}

impl Display for ValidBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ValidBlock({})", self.block)
    }
}
