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

use serde::Serialize;
use tari_dan_common_types::{Epoch, NodeHeight};

use crate::{
    consensus_models::{Block, BlockId, LeafBlock, QcId, QuorumCertificate},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HighQc {
    pub block_id: BlockId,
    pub block_height: NodeHeight,
    pub epoch: Epoch,
    pub qc_id: QcId,
}

impl HighQc {
    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }

    pub fn block_height(&self) -> NodeHeight {
        self.block_height
    }

    pub fn qc_id(&self) -> &QcId {
        &self.qc_id
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn as_leaf_block(&self) -> LeafBlock {
        LeafBlock {
            block_id: self.block_id,
            height: self.block_height,
            epoch: self.epoch,
        }
    }
}

impl HighQc {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &TTx, epoch: Epoch) -> Result<Self, StorageError> {
        tx.high_qc_get(epoch)
    }

    pub fn get_quorum_certificate<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<QuorumCertificate, StorageError> {
        QuorumCertificate::get(tx, &self.qc_id)
    }

    pub fn get_block<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<Block, StorageError> {
        Block::get(tx, &self.block_id)
    }

    pub fn set<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.high_qc_set(self)
    }
}

impl Display for HighQc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(block_id: {}, height: {}, qc_id: {})",
            self.block_id, self.block_height, self.qc_id
        )
    }
}
