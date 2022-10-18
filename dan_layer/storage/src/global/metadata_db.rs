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

use crate::global::GlobalDbAdapter;

pub struct MetadataDb<'a, TGlobalDbAdapter: GlobalDbAdapter> {
    backend: &'a TGlobalDbAdapter,
    tx: &'a TGlobalDbAdapter::DbTransaction,
}

impl<'a, TGlobalDbAdapter: GlobalDbAdapter> MetadataDb<'a, TGlobalDbAdapter> {
    pub fn new(backend: &'a TGlobalDbAdapter, tx: &'a TGlobalDbAdapter::DbTransaction) -> Self {
        Self { backend, tx }
    }

    pub fn set_metadata(&self, key: MetadataKey, value: &[u8]) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend.set_metadata(self.tx, key, value)?;
        Ok(())
    }

    pub fn get_metadata(&self, key: MetadataKey) -> Result<Option<Vec<u8>>, TGlobalDbAdapter::Error> {
        let data = self.backend.get_metadata(self.tx, &key)?;
        Ok(data)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MetadataKey {
    BaseLayerScannerLastScannedBlockHeight,
    BaseLayerScannerLastScannedBlockHash,
    CurrentEpoch,
    NextEpochRegistration,
}

impl MetadataKey {
    pub fn as_key_bytes(self) -> &'static [u8] {
        match self {
            MetadataKey::BaseLayerScannerLastScannedBlockHash => b"base_layer_scanner.last_scanned_block_hash",
            MetadataKey::BaseLayerScannerLastScannedBlockHeight => b"base_layer_scanner.last_scanned_block_height",
            MetadataKey::CurrentEpoch => b"current_epoch",
            MetadataKey::NextEpochRegistration => b"last_registered_epoch",
        }
    }
}
