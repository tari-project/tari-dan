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

use serde::{Serialize, de::DeserializeOwned};

use crate::global::GlobalDbAdapter;

pub struct MetadataDb<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> {
    backend: &'a TGlobalDbAdapter,
    tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
}

impl<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> MetadataDb<'a, 'tx, TGlobalDbAdapter> {
    pub fn new(backend: &'a TGlobalDbAdapter, tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>) -> Self {
        Self { backend, tx }
    }

    pub fn set_metadata<T: Serialize>(&mut self, key: &[u8], value: &T) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend.set_metadata(self.tx, key, value)?;
        Ok(())
    }

    pub fn get_metadata<T: DeserializeOwned>(&mut self, key: &[u8]) -> Result<Option<T>, TGlobalDbAdapter::Error> {
        let data = self.backend.get_metadata(self.tx, key)?;
        Ok(data)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MetadataKey {
    EpochManagerCurrentEpoch,
    EpochManagerCurrentShardKey,
    EpochManagerLastEpochHash,
    EpochManagerLastSyncedEpoch,
    EpochManagerFeeClaimPublicKey,
    /// Highest epoch whose hash has been locked in by a committed EndEpoch block.
    /// Epochs <= this value can no longer have their hash corrected by the oracle.
    EpochManagerHighestLockedEpoch,
    /// The "birthday" epoch of the network
    EpochManagerBirthdayEpoch,
    /// The schema activation schedule the node last started with. Compared against the running
    /// binary's schedule to detect a disagreement about activations the node has already passed.
    ProtocolActivationSchedule,
    /// The genesis protocol version the node last started with. A network that has not launched may
    /// have its genesis version changed; this catches a node with history whose binary makes that
    /// change under it. Absent means V0, which is what every binary predating this key ran under.
    ProtocolGenesisVersion,
}

impl MetadataKey {
    pub const fn as_key_bytes(self) -> &'static [u8] {
        match self {
            Self::EpochManagerCurrentEpoch => b"epoch_manager.current_epoch",
            Self::EpochManagerCurrentShardKey => b"epoch_manager.current_shard_key",
            Self::EpochManagerLastSyncedEpoch => b"epoch_manager.last_synced_epoch",
            Self::EpochManagerFeeClaimPublicKey => b"epoch_manager.fee_claim_public_key",
            Self::EpochManagerLastEpochHash => b"epoch_manager.last_epoch_hash",
            Self::EpochManagerHighestLockedEpoch => b"epoch_manager.highest_locked_epoch",
            Self::EpochManagerBirthdayEpoch => b"epoch_manager.birthday_epoch",
            Self::ProtocolActivationSchedule => b"protocol.activation_schedule",
            Self::ProtocolGenesisVersion => b"protocol.genesis_version",
        }
    }
}
