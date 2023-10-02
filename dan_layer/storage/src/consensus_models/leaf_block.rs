//  Copyright 2022, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::fmt::Display;

use tari_dan_common_types::NodeHeight;

use crate::{
    consensus_models::{Block, BlockId},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeafBlock {
    pub block_id: BlockId,
    pub height: NodeHeight,
}

impl LeafBlock {
    pub fn is_genesis(&self) -> bool {
        self.block_id.is_genesis()
    }

    pub fn genesis() -> Self {
        Self {
            block_id: BlockId::genesis(),
            height: NodeHeight(0),
        }
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }
}

impl LeafBlock {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx) -> Result<Self, StorageError> {
        tx.leaf_block_get()
    }

    pub fn set<TTx: StateStoreWriteTransaction + ?Sized>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.leaf_block_set(self)
    }

    pub fn unset<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.leaf_block_unset(self)
    }

    pub fn get_block<TTx: StateStoreReadTransaction>(&self, tx: &mut TTx) -> Result<Block<TTx::Addr>, StorageError> {
        tx.blocks_get(&self.block_id)
    }
}

impl Display for LeafBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} #{}", self.block_id, self.height)
    }
}
