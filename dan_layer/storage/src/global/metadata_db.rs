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

use serde::{de::DeserializeOwned, Serialize};

use crate::global::GlobalDbAdapter;

pub struct MetadataDb<'a, TGlobalDbAdapter: GlobalDbAdapter> {
    backend: &'a TGlobalDbAdapter,
    tx: &'a TGlobalDbAdapter::DbTransaction<'a>,
}

impl<'a, TGlobalDbAdapter: GlobalDbAdapter> MetadataDb<'a, TGlobalDbAdapter> {
    pub fn new(backend: &'a TGlobalDbAdapter, tx: &'a TGlobalDbAdapter::DbTransaction<'a>) -> Self {
        Self { backend, tx }
    }

    pub fn set_metadata<T: Serialize>(&self, key: MetadataKey, value: &T) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend.set_metadata(self.tx, key, value)?;
        Ok(())
    }

    pub fn get_metadata<T: DeserializeOwned>(&self, key: MetadataKey) -> Result<Option<T>, TGlobalDbAdapter::Error> {
        let data = self.backend.get_metadata(self.tx, &key)?;
        Ok(data)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MetadataKey {
    BaseLayerScannerLastScannedTip,
    BaseLayerScannerLastScannedBlockHeight,
    BaseLayerScannerLastScannedBlockHash,
    BaseLayerScannerNextBlockHash,
    BaseLayerConsensusConstants,
    CurrentEpoch,
    CurrentShardKey,
    LastEpochRegistration,
    LastSyncedEpoch,
}

impl MetadataKey {
    pub fn as_key_bytes(self) -> &'static [u8] {
        match self {
            MetadataKey::BaseLayerScannerLastScannedTip => b"base_layer_scanner.last_scanned_tip",
            MetadataKey::BaseLayerScannerLastScannedBlockHash => b"base_layer_scanner.last_scanned_block_hash",
            MetadataKey::BaseLayerScannerLastScannedBlockHeight => b"base_layer_scanner.last_scanned_block_height",
            MetadataKey::BaseLayerScannerNextBlockHash => b"base_layer_scanner.next_block_hash",
            MetadataKey::BaseLayerConsensusConstants => b"base_layer.consensus_constants",
            MetadataKey::CurrentEpoch => b"current_epoch",
            MetadataKey::LastEpochRegistration => b"last_registered_epoch",
            MetadataKey::CurrentShardKey => b"current_shard_key",
            MetadataKey::LastSyncedEpoch => b"last_synced_epoch",
        }
    }
}
