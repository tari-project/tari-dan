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

use tari_common_types::types::FixedHash;
use tari_ootle_common_types::{Epoch, ShardGroup, ValidatorSetHash};
use tari_template_lib_types::Hash32;

use crate::global::GlobalDbAdapter;

pub struct EpochDb<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> {
    backend: &'a TGlobalDbAdapter,
    tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
}

impl<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> EpochDb<'a, 'tx, TGlobalDbAdapter> {
    pub fn new(backend: &'a TGlobalDbAdapter, tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>) -> Self {
        Self { backend, tx }
    }

    pub fn insert_epoch(&mut self, epoch: Epoch, epoch_hash: FixedHash) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend.insert_epoch(self.tx, epoch, epoch_hash)
    }

    pub fn get_epoch_data(&mut self, epoch: Epoch) -> Result<Option<EpochData>, TGlobalDbAdapter::Error> {
        self.backend.get_epoch(self.tx, epoch)
    }

    pub fn set_validator_set(
        &mut self,
        epoch: Epoch,
        validator_set_hash: &ValidatorSetHash,
        leaves: &[(ShardGroup, Hash32)],
    ) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend
            .set_validator_set(self.tx, epoch, validator_set_hash, leaves)
    }

    pub fn get_validator_set_leaves(
        &mut self,
        epoch: Epoch,
    ) -> Result<Vec<(ShardGroup, Hash32)>, TGlobalDbAdapter::Error> {
        self.backend.get_validator_set_leaves(self.tx, epoch)
    }
}

#[derive(Debug, Clone)]
pub struct EpochData {
    pub epoch: u64,
    pub epoch_hash: FixedHash,
    /// `None` until the epoch manager stores the hash, which it does each time it assigns the epoch's committees.
    pub validator_set_hash: Option<ValidatorSetHash>,
}
