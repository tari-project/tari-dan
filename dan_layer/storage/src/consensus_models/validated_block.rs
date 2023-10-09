//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, fmt::Display, ops::DerefMut};

use tari_dan_common_types::{Epoch, NodeAddressable, NodeHeight};

use crate::{
    consensus_models::{Block, BlockId},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

pub struct ValidBlock<TAddr> {
    block: Block<TAddr>,
    dummy_blocks: Vec<Block<TAddr>>,
}

impl<TAddr> ValidBlock<TAddr> {
    pub fn new(block: Block<TAddr>) -> Self {
        Self {
            block,
            dummy_blocks: vec![],
        }
    }

    pub fn with_dummy_blocks(block: Block<TAddr>, dummy_blocks: Vec<Block<TAddr>>) -> Self {
        Self { block, dummy_blocks }
    }

    pub fn block(&self) -> &Block<TAddr> {
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

    pub fn proposed_by(&self) -> &TAddr {
        self.block.proposed_by()
    }

    pub fn dummy_blocks(&self) -> &[Block<TAddr>] {
        &self.dummy_blocks
    }
}

impl<TAddr: NodeAddressable> ValidBlock<TAddr> {
    pub fn save_all_dummy_blocks<TTx>(&self, tx: &mut TTx) -> Result<(), StorageError>
    where
        TTx: StateStoreWriteTransaction<Addr = TAddr> + DerefMut,
        TTx::Target: StateStoreReadTransaction<Addr = TAddr>,
    {
        for block in &self.dummy_blocks {
            block.save(tx)?;
        }
        Ok(())
    }
}

impl<TAddr: NodeAddressable> Display for ValidBlock<TAddr> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ValidatedBlock({})", self.block)
    }
}
