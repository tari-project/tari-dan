//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Borrow, collections::HashSet, ops::RangeInclusive};

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{optional::Optional, Epoch, NodeHeight, ShardId};
use tari_engine_types::substate::{Substate, SubstateAddress, SubstateValue};
use tari_transaction::TransactionId;

use crate::{
    consensus_models::{BlockId, QcId, QuorumCertificate},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstateRecord {
    pub address: SubstateAddress,
    pub version: u32,
    pub substate_value: SubstateValue,
    pub state_hash: FixedHash,
    pub created_by_transaction: TransactionId,
    pub created_justify: QcId,
    pub created_block: BlockId,
    pub created_height: NodeHeight,
    pub destroyed_by_transaction: Option<TransactionId>,
    pub destroyed_justify: Option<QcId>,
    pub destroyed_by_block: Option<BlockId>,
    pub created_at_epoch: Epoch,
    pub destroyed_at_epoch: Option<Epoch>,
}

impl SubstateRecord {
    pub fn new(
        address: SubstateAddress,
        version: u32,
        substate_value: SubstateValue,
        created_at_epoch: Epoch,
        created_height: NodeHeight,
        created_block: BlockId,
        created_by_transaction: TransactionId,
        created_justify: QcId,
    ) -> Self {
        Self {
            address,
            version,
            substate_value,
            state_hash: Default::default(),
            created_height,
            created_justify,
            destroyed_justify: None,
            destroyed_by_block: None,
            created_at_epoch,
            destroyed_at_epoch: None,
            created_by_transaction,
            created_block,
            destroyed_by_transaction: None,
        }
    }

    pub fn to_shard_id(&self) -> ShardId {
        ShardId::from_address(&self.address, self.version)
    }

    pub fn substate_address(&self) -> &SubstateAddress {
        &self.address
    }

    pub fn substate_value(&self) -> &SubstateValue {
        &self.substate_value
    }

    pub fn into_substate_value(self) -> SubstateValue {
        self.substate_value
    }

    pub fn to_substate(&self) -> Substate {
        Substate::new(self.version, self.substate_value.clone())
    }

    pub fn into_substate(self) -> Substate {
        Substate::new(self.version, self.substate_value)
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn created_height(&self) -> NodeHeight {
        self.created_height
    }

    pub fn destroyed_by_block(&self) -> Option<BlockId> {
        self.destroyed_by_block
    }

    pub fn created_block(&self) -> BlockId {
        self.created_block
    }

    pub fn created_by_transaction(&self) -> TransactionId {
        self.created_by_transaction
    }

    pub fn destroyed_by_transaction(&self) -> Option<TransactionId> {
        self.destroyed_by_transaction
    }

    pub fn created_justify(&self) -> &QcId {
        &self.created_justify
    }

    pub fn destroyed_justify(&self) -> Option<&QcId> {
        self.destroyed_justify.as_ref()
    }

    pub fn is_destroyed(&self) -> bool {
        self.destroyed_by_transaction.is_some()
    }
}

impl SubstateRecord {
    pub fn try_lock_many<'a, TTx: StateStoreWriteTransaction, I: IntoIterator<Item = &'a ShardId>>(
        tx: &mut TTx,
        locked_by_tx: &TransactionId,
        inputs: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<SubstateLockState, StorageError> {
        tx.substates_try_lock_many(locked_by_tx, inputs, lock_flag)
    }

    pub fn try_unlock_many<'a, TTx: StateStoreWriteTransaction, I: IntoIterator<Item = &'a ShardId>>(
        tx: &mut TTx,
        locked_by_tx: &TransactionId,
        inputs: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<(), StorageError> {
        tx.substates_try_unlock_many(locked_by_tx, inputs, lock_flag)?;
        Ok(())
    }

    pub fn create<TTx: StateStoreWriteTransaction>(self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.substates_create(self)?;
        Ok(())
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &mut TTx,
        shard: &ShardId,
    ) -> Result<bool, StorageError> {
        Ok(Self::get(tx, shard).optional()?.is_some())
    }

    pub fn get<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &mut TTx,
        shard: &ShardId,
    ) -> Result<SubstateRecord, StorageError> {
        tx.substates_get(shard)
    }

    pub fn get_any<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a ShardId>>(
        tx: &mut TTx,
        shards: I,
    ) -> Result<(Vec<SubstateRecord>, HashSet<ShardId>), StorageError> {
        let mut shards = shards.into_iter().copied().collect::<HashSet<_>>();
        let found = tx.substates_get_any(&shards)?;
        for f in &found {
            shards.remove(&f.to_shard_id());
        }

        Ok((found, shards))
    }

    pub fn get_many_within_range<TTx: StateStoreReadTransaction, B: Borrow<RangeInclusive<ShardId>>>(
        tx: &mut TTx,
        bounds: B,
        excluded_shards: &[ShardId],
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        tx.substates_get_many_within_range(bounds.borrow().start(), bounds.borrow().end(), excluded_shards)
    }

    pub fn get_many_by_created_transaction<TTx: StateStoreReadTransaction>(
        tx: &mut TTx,
        transaction_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        tx.substates_get_many_by_created_transaction(transaction_id)
    }

    pub fn get_many_by_destroyed_transaction<TTx: StateStoreReadTransaction>(
        tx: &mut TTx,
        transaction_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        tx.substates_get_many_by_destroyed_transaction(transaction_id)
    }

    pub fn get_created_quorum_certificate<TTx: StateStoreReadTransaction>(
        &self,
        tx: &mut TTx,
    ) -> Result<QuorumCertificate<TTx::Addr>, StorageError> {
        tx.quorum_certificates_get(self.created_justify())
    }

    pub fn get_destroyed_quorum_certificate<TTx: StateStoreReadTransaction>(
        &self,
        tx: &mut TTx,
    ) -> Result<Option<QuorumCertificate<TTx::Addr>>, StorageError> {
        self.destroyed_justify()
            .map(|justify| tx.quorum_certificates_get(justify))
            .transpose()
    }
}

/// Substate lock flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SubstateLockFlag {
    Read = 0x01,
    Write = 0x02,
}

pub enum SubstateLockState {
    SomeWriteLocked,
    SomeReadLocked,
    LockAcquired,
}

impl SubstateLockState {
    pub fn is_acquired(&self) -> bool {
        matches!(self, Self::LockAcquired)
    }
}
