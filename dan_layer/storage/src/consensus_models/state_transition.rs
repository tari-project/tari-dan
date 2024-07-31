//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Display, Formatter},
    io::{Read, Write},
    mem,
};

use tari_dan_common_types::{shard::Shard, Epoch};
use tari_state_tree::Version;

use crate::{consensus_models::SubstateUpdate, StateStoreReadTransaction, StorageError};

#[derive(Debug, Clone)]
pub struct StateTransition {
    pub id: StateTransitionId,
    pub update: SubstateUpdate,
    pub state_tree_version: Version,
}

impl StateTransition {
    pub fn get_n_after<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        n: usize,
        after_id: StateTransitionId,
        end_epoch: Epoch,
    ) -> Result<Vec<Self>, StorageError> {
        tx.state_transitions_get_n_after(n, after_id, end_epoch)
    }

    pub fn get_last_id<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        shard: Shard,
    ) -> Result<StateTransitionId, StorageError> {
        tx.state_transitions_get_last_id(shard)
    }
}

impl Display for StateTransition {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.id, self.update)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateTransitionId {
    epoch: Epoch,
    shard: Shard,
    seq: u64,
}
impl StateTransitionId {
    const BYTE_SIZE: usize = mem::size_of::<Self>();

    pub fn new(epoch: Epoch, shard: Shard, seq: u64) -> Self {
        Self { epoch, shard, seq }
    }

    pub fn initial(shard: Shard) -> Self {
        Self::new(Epoch(1), shard, 0)
    }

    pub fn from_bytes(mut bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::BYTE_SIZE {
            return None;
        }
        let bytes_mut = &mut bytes;
        let epoch = Epoch(u64::from_le_bytes(copy_fixed(bytes_mut)));
        let shard = Shard::from(u32::from_le_bytes(copy_fixed(bytes_mut)));
        let seq = u64::from_le_bytes(copy_fixed(bytes_mut));
        Some(Self::new(epoch, shard, seq))
    }

    pub fn as_bytes(&self) -> [u8; Self::BYTE_SIZE] {
        let mut buf = [0u8; Self::BYTE_SIZE];
        let buf_mut = &mut buf.as_mut_slice();
        write_fixed(self.epoch.to_le_bytes(), buf_mut);
        write_fixed(self.shard.as_u32().to_le_bytes(), buf_mut);
        write_fixed(self.seq.to_le_bytes(), buf_mut);
        buf
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn shard(&self) -> Shard {
        self.shard
    }

    pub fn seq(self) -> u64 {
        self.seq
    }
}

impl Display for StateTransitionId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StateTransition({}, {}, seq = {})",
            self.epoch(),
            self.shard(),
            self.seq()
        )
    }
}

/// Copies bytes into a fixed byte array.
///
/// ## Panics
/// Caller must ensure that sufficient bytes remain on the mut ref to the input slice.
fn copy_fixed<const SZ: usize>(bytes: &mut &[u8]) -> [u8; SZ] {
    let mut buf = [0u8; SZ];
    bytes
        .read_exact(&mut buf)
        .expect("copy_fixed: Expected enough bytes to read");
    buf
}

/// Writes fixed bytes into a buffer.
/// ## Panics
/// Caller must ensure that the buffer has sufficient space for the fixed bytes.
fn write_fixed<const SZ: usize>(buf: [u8; SZ], out: &mut &mut [u8]) {
    out.write_all(&buf)
        .expect("write_fixed: Expected buffer to have sufficient space for fixed bytes");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_and_from_bytes() {
        let id = StateTransitionId::new(Epoch(1), Shard::from(2), 3);
        let bytes = id.as_bytes();
        let id2 = StateTransitionId::from_bytes(&bytes).unwrap();
        assert_eq!(id, id2);

        assert_eq!(StateTransitionId::from_bytes(&[1, 2, 3]), None);
    }
}
