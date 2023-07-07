//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::ShardId;
use tari_engine_types::substate::SubstateAddress;

use crate::{
    consensus_models::{BlockId, QcId, TransactionId},
    StateStoreWriteTransaction,
    StorageError,
};

/// Substate lock flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SubstateLockFlag {
    Read = 0x01,
    Write = 0x02,
}

#[derive(Debug, Clone)]
pub struct SubstateRecord {
    pub shard_id: ShardId,
    pub address: SubstateAddress,
    pub version: u64,
    pub data: Vec<u8>,
    pub state_hash: FixedHash,
    pub created_by_transaction: TransactionId,
    pub created_justify: Option<QcId>,
    pub created_block: BlockId,
    pub created_height: u64,
    pub destroyed_by_transaction: Option<TransactionId>,
    pub destroyed_justify: Option<QcId>,
    pub destroyed_by_block: Option<BlockId>,
    pub fee_paid_for_created_justify: u64,
    pub fee_paid_for_deleted_justify: u64,
    pub created_at_epoch: Option<u64>,
    pub destroyed_at_epoch: Option<u64>,
    pub created_justify_leader: Option<PublicKey>,
    pub destroyed_justify_leader: Option<PublicKey>,
    pub read_locks: u32,
    pub is_locked_w: bool,
}

impl SubstateRecord {
    pub fn try_lock_many<'a, TTx: StateStoreWriteTransaction, I: IntoIterator<Item = &'a ShardId>>(
        tx: &mut TTx,
        inputs: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<(), StorageError> {
        tx.substates_try_lock_many(inputs, lock_flag)?;
        Ok(())
    }

    pub fn try_unlock_many<'a, TTx: StateStoreWriteTransaction, I: IntoIterator<Item = &'a ShardId>>(
        tx: &mut TTx,
        inputs: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<(), StorageError> {
        tx.substates_try_unlock_many(inputs, lock_flag)?;
        Ok(())
    }
}
