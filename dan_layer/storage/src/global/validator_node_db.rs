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

use tari_dan_common_types::Epoch;

use crate::global::{models::ValidatorNode, GlobalDbAdapter};

pub struct ValidatorNodeDb<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> {
    backend: &'a TGlobalDbAdapter,
    tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
}

impl<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> ValidatorNodeDb<'a, 'tx, TGlobalDbAdapter> {
    pub fn new(backend: &'a TGlobalDbAdapter, tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>) -> Self {
        Self { backend, tx }
    }

    pub fn insert_validator_nodes(
        &mut self,
        validator_nodes: Vec<ValidatorNode>,
    ) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend
            .insert_validator_nodes(self.tx, validator_nodes)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn count(&mut self, start_epoch: Epoch, end_epoch: Epoch) -> Result<u64, TGlobalDbAdapter::Error> {
        self.backend
            .count_validator_nodes(self.tx, start_epoch, end_epoch)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn get(
        &mut self,
        start_epoch: Epoch,
        end_epoch: Epoch,
        public_key: &[u8],
    ) -> Result<ValidatorNode, TGlobalDbAdapter::Error> {
        self.backend
            .get_validator_node(self.tx, start_epoch, end_epoch, public_key)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn get_all_within_epochs(
        &mut self,
        start_epoch: Epoch,
        end_epoch: Epoch,
    ) -> Result<Vec<ValidatorNode>, TGlobalDbAdapter::Error> {
        self.backend
            .get_validator_nodes_within_epochs(self.tx, start_epoch, end_epoch)
            .map_err(TGlobalDbAdapter::Error::into)
    }
}
