//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_ootle_common_types::Epoch;
use tari_sidechain::FixedHash;
use tari_template_lib_types::Hash32;

use crate::global::GlobalDbAdapter;

pub struct BlockHeaderDb<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> {
    backend: &'a TGlobalDbAdapter,
    tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
}

impl<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> BlockHeaderDb<'a, 'tx, TGlobalDbAdapter> {
    pub fn new(backend: &'a TGlobalDbAdapter, tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>) -> Self {
        Self { backend, tx }
    }

    pub fn insert(&mut self, header: BlockHeaderModel) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend.insert_block_header(self.tx, header)
    }

    pub fn get_by_hash(&mut self, epoch: Epoch, hash: &Hash32) -> Result<BlockHeaderModel, TGlobalDbAdapter::Error> {
        self.backend.get_block_header_by_hash(self.tx, epoch, hash)
    }

    pub fn get_first_block_header_in_epoch(
        &mut self,
        epoch: Epoch,
    ) -> Result<BlockHeaderModel, TGlobalDbAdapter::Error> {
        self.backend.get_first_block_header_by_epoch(self.tx, epoch)
    }

    /// Deletes every stored header with a height strictly greater than `height`, returning the number of
    /// rows removed. Used to discard headers orphaned by a base-layer reorg.
    pub fn delete_above(&mut self, height: u64) -> Result<usize, TGlobalDbAdapter::Error> {
        self.backend.delete_block_headers_above(self.tx, height)
    }
}

#[derive(Debug, Clone)]
pub struct BlockHeaderModel {
    pub epoch: Epoch,
    pub height: u64,
    pub block_hash: FixedHash,
    pub block_output_merkle_root: FixedHash,
    pub validator_node_merkle_root: FixedHash,
}
