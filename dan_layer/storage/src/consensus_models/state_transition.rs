//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Display, Formatter},
    mem::size_of,
};

use tari_dan_common_types::{shard::Shard, Epoch};

use crate::{consensus_models::SubstateUpdate, StateStoreReadTransaction, StorageError};

#[derive(Debug, Clone)]
pub struct StateTransition {
    pub id: StateTransitionId,
    pub update: SubstateUpdate,
}

impl StateTransition {
    pub fn get_n_after<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        n: usize,
        after_id: StateTransitionId,
    ) -> Result<Vec<Self>, StorageError> {
        tx.state_transitions_get_n_after(n, after_id)
    }
}

/// 20 byte ID
/// epoch: Epoch,
/// shard: Shard,
/// seq_no: u64,
#[derive(Debug, Clone, Copy)]
pub struct StateTransitionId([u8; 20]);

const U64_SZ: usize = size_of::<u64>();
const U32_SZ: usize = size_of::<u32>();

impl StateTransitionId {
    pub fn from_parts(epoch: Epoch, shard: Shard, seq: u64) -> Self {
        let mut buf = [0u8; 20];
        buf[..U64_SZ].copy_from_slice(&epoch.as_u64().to_le_bytes());
        buf[U64_SZ..U64_SZ + U32_SZ].copy_from_slice(&shard.as_u32().to_le_bytes());
        buf[U64_SZ + U32_SZ..].copy_from_slice(&seq.to_le_bytes());
        Self(buf)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn to_epoch(&self) -> Epoch {
        let mut buf = [0u8; size_of::<u64>()];
        buf.copy_from_slice(&self.0[..U64_SZ]);
        Epoch(u64::from_le_bytes(buf))
    }

    pub fn to_shard(&self) -> Shard {
        let mut buf = [0u8; U32_SZ];
        buf.copy_from_slice(&self.0[U64_SZ..U64_SZ + U32_SZ]);
        Shard::from(u32::from_le_bytes(buf))
    }

    pub fn to_seq(self) -> u64 {
        let mut buf = [0u8; U64_SZ];
        buf.copy_from_slice(&self.0[U64_SZ + U32_SZ..]);
        u64::from_le_bytes(buf)
    }
}

impl Display for StateTransitionId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for b in self.0 {
            write!(f, "{:02x?}", b)?;
        }
        write!(
            f,
            " (epoch = {}, shard = {}, seq = {})",
            self.to_epoch(),
            self.to_shard(),
            self.to_seq()
        )
    }
}
