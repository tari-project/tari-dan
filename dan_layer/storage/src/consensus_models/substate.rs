//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Borrow, collections::HashSet, fmt, fmt::Display, iter, iter::Peekable, ops::RangeInclusive};

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{
    shard::Shard,
    Epoch,
    NodeHeight,
    SubstateAddress,
    SubstateRequirement,
    VersionedSubstateId,
};
use tari_engine_types::substate::{hash_substate, Substate, SubstateId, SubstateValue};
use tari_transaction::TransactionId;

use crate::{
    consensus_models::{BlockId, QcId, QuorumCertificate, SubstateLock},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct SubstateRecord {
    pub substate_id: SubstateId,
    pub version: u32,
    pub substate_value: SubstateValue,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub state_hash: FixedHash,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub created_by_transaction: TransactionId,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub created_justify: QcId,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub created_block: BlockId,
    pub created_height: NodeHeight,
    pub created_by_shard: Shard,
    pub created_at_epoch: Epoch,
    pub destroyed: Option<SubstateDestroyed>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct SubstateDestroyed {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub by_transaction: TransactionId,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub justify: QcId,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub by_block: NodeHeight,
    pub at_epoch: Epoch,
    pub by_shard: Shard,
}

impl SubstateRecord {
    pub fn new(
        substate_id: SubstateId,
        version: u32,
        substate_value: SubstateValue,
        created_by_shard: Shard,
        created_at_epoch: Epoch,
        created_height: NodeHeight,
        created_block: BlockId,
        created_by_transaction: TransactionId,
        created_justify: QcId,
    ) -> Self {
        Self {
            substate_id,
            version,
            state_hash: hash_substate(&substate_value, version),
            substate_value,
            created_height,
            created_justify,
            created_by_shard,
            created_at_epoch,
            created_by_transaction,
            created_block,
            destroyed: None,
        }
    }

    pub fn to_substate_address(&self) -> SubstateAddress {
        SubstateAddress::from_substate_id(&self.substate_id, self.version)
    }

    pub fn to_versioned_substate_id(&self) -> VersionedSubstateId {
        VersionedSubstateId::new(self.substate_id.clone(), self.version)
    }

    pub fn to_substate_requirement(&self) -> SubstateRequirement {
        SubstateRequirement::with_version(self.substate_id.clone(), self.version)
    }

    pub fn substate_id(&self) -> &SubstateId {
        &self.substate_id
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

    pub fn created_block(&self) -> BlockId {
        self.created_block
    }

    pub fn created_by_transaction(&self) -> TransactionId {
        self.created_by_transaction
    }

    pub fn created_justify(&self) -> &QcId {
        &self.created_justify
    }

    pub fn destroyed(&self) -> Option<&SubstateDestroyed> {
        self.destroyed.as_ref()
    }

    pub fn is_destroyed(&self) -> bool {
        self.destroyed.is_some()
    }

    pub fn is_up(&self) -> bool {
        !self.is_destroyed()
    }
}

impl SubstateRecord {
    pub fn lock_all<TTx: StateStoreWriteTransaction, I: IntoIterator<Item = (SubstateId, Vec<SubstateLock>)>>(
        tx: &mut TTx,
        block_id: BlockId,
        locks: I,
    ) -> Result<(), StorageError> {
        tx.substate_locks_insert_all(block_id, locks)
    }

    pub fn unlock_all<'a, TTx: StateStoreWriteTransaction, I: Iterator<Item = &'a TransactionId>>(
        tx: &mut TTx,
        transaction_ids: Peekable<I>,
    ) -> Result<(), StorageError> {
        tx.substate_locks_remove_many_for_transactions(transaction_ids)
    }

    pub fn create<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.substates_create(self)?;
        Ok(())
    }

    pub fn exists<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        id: &VersionedSubstateId,
    ) -> Result<bool, StorageError> {
        Self::any_exist(tx, Some(id))
    }

    pub fn any_exist<
        TTx: StateStoreReadTransaction + ?Sized,
        I: IntoIterator<Item = S>,
        S: Borrow<VersionedSubstateId>,
    >(
        tx: &TTx,
        substates: I,
    ) -> Result<bool, StorageError> {
        tx.substates_any_exist(substates)
    }

    pub fn exists_for_transaction<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        transaction_id: &TransactionId,
    ) -> Result<bool, StorageError> {
        tx.substates_exists_for_transaction(transaction_id)
    }

    pub fn get<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        address: &SubstateAddress,
    ) -> Result<SubstateRecord, StorageError> {
        tx.substates_get(address)
    }

    pub fn substate_is_up<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        shard: &SubstateAddress,
    ) -> Result<bool, StorageError> {
        // TODO: consider optimising
        let rec = tx.substates_get(shard)?;
        Ok(rec.is_up())
    }

    pub fn get_any<TTx: StateStoreReadTransaction + ?Sized, I: IntoIterator<Item = SubstateRequirement>>(
        tx: &TTx,
        shards: I,
    ) -> Result<(Vec<SubstateRecord>, HashSet<SubstateRequirement>), StorageError> {
        let mut substate_ids = shards.into_iter().collect::<HashSet<_>>();
        let found = tx.substates_get_any(&substate_ids)?;
        for f in &found {
            substate_ids.remove(&f.to_substate_requirement());
        }

        Ok((found, substate_ids))
    }

    pub fn get_any_max_version<'a, TTx: StateStoreReadTransaction + ?Sized, I: IntoIterator<Item = &'a SubstateId>>(
        tx: &TTx,
        substate_ids: I,
    ) -> Result<(Vec<SubstateRecord>, HashSet<&'a SubstateId>), StorageError> {
        let mut substate_ids = substate_ids.into_iter().collect::<HashSet<_>>();
        let found = tx.substates_get_any_max_version(substate_ids.iter().copied())?;
        for f in &found {
            substate_ids.remove(&f.substate_id);
        }

        Ok((found, substate_ids))
    }

    pub fn get_latest_version<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        substate_id: &SubstateId,
    ) -> Result<(u32, bool), StorageError> {
        tx.substates_get_max_version_for_substate(substate_id)
    }

    pub fn get_latest<TTx: StateStoreReadTransaction + ?Sized>(
        tx: &TTx,
        substate_id: &SubstateId,
    ) -> Result<SubstateRecord, StorageError> {
        // TODO: consider optimising
        let (mut found, _) = Self::get_any_max_version(tx, iter::once(substate_id))?;
        let Some(found) = found.pop() else {
            return Err(StorageError::NotFound {
                item: "SubstateRecord::get_latest".to_string(),
                key: substate_id.to_string(),
            });
        };

        Ok(found)
    }

    pub fn get_n_after<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        n: usize,
        after: &SubstateAddress,
    ) -> Result<Vec<Self>, StorageError> {
        tx.substates_get_n_after(n, after)
    }

    pub fn get_many_within_range<TTx: StateStoreReadTransaction, B: Borrow<RangeInclusive<SubstateAddress>>>(
        tx: &TTx,
        bounds: B,
        excluded_shards: &[SubstateAddress],
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        tx.substates_get_many_within_range(bounds.borrow().start(), bounds.borrow().end(), excluded_shards)
    }

    pub fn get_many_by_created_transaction<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        transaction_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        tx.substates_get_many_by_created_transaction(transaction_id)
    }

    pub fn get_many_by_destroyed_transaction<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        transaction_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        tx.substates_get_many_by_destroyed_transaction(transaction_id)
    }

    pub fn get_created_quorum_certificate<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<QuorumCertificate, StorageError> {
        tx.quorum_certificates_get(self.created_justify())
    }

    pub fn get_destroyed_quorum_certificate<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<Option<QuorumCertificate>, StorageError> {
        self.destroyed()
            .map(|destroyed| tx.quorum_certificates_get(&destroyed.justify))
            .transpose()
    }

    pub fn destroy<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        versioned_substate_id: VersionedSubstateId,
        shard: Shard,
        epoch: Epoch,
        destroyed_by_block: NodeHeight,
        destroyed_justify: &QcId,
        destroyed_by_transaction: &TransactionId,
    ) -> Result<(), StorageError> {
        tx.substates_down(
            versioned_substate_id,
            shard,
            epoch,
            destroyed_by_block,
            destroyed_by_transaction,
            destroyed_justify,
        )
    }
}

#[derive(Debug, Clone)]
pub struct SubstateCreatedProof {
    pub substate: SubstateData,
    // TODO: proof that data was created
}

#[derive(Debug, Clone)]
pub struct SubstateDestroyedProof {
    pub substate_id: SubstateId,
    pub version: u32,
    // TODO: proof that data was destroyed
    pub destroyed_by_transaction: TransactionId,
}

#[derive(Debug, Clone)]
pub struct SubstateData {
    pub substate_id: SubstateId,
    pub version: u32,
    pub substate_value: SubstateValue,
    pub created_by_transaction: TransactionId,
}

impl SubstateData {
    pub fn into_substate(self) -> Substate {
        Substate::new(self.version, self.substate_value)
    }
}

impl From<SubstateRecord> for SubstateData {
    fn from(value: SubstateRecord) -> Self {
        Self {
            substate_id: value.substate_id,
            version: value.version,
            substate_value: value.substate_value,
            created_by_transaction: value.created_by_transaction,
        }
    }
}

#[derive(Debug, Clone)]
pub enum SubstateUpdate {
    Create(SubstateCreatedProof),
    Destroy(SubstateDestroyedProof),
}

impl SubstateUpdate {
    pub fn is_create(&self) -> bool {
        matches!(self, Self::Create(_))
    }

    pub fn is_destroy(&self) -> bool {
        matches!(self, Self::Destroy { .. })
    }
}

impl From<SubstateCreatedProof> for SubstateUpdate {
    fn from(value: SubstateCreatedProof) -> Self {
        Self::Create(value)
    }
}

impl Display for SubstateUpdate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Create(proof) => write!(f, "Create: {}(v{})", proof.substate.substate_id, proof.substate.version),
            Self::Destroy(proof) => write!(f, "Destroy: {}(v{})", proof.substate_id, proof.version),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SubstateLockState {
    /// The lock was successfully acquired
    LockAcquired,
    /// The lock was not acquired because some substates are DOWN
    SomeDestroyed,
    /// Some substates are locked for write
    SomeAlreadyWriteLocked,
    /// Some outputs substates exist. This indicates that that we attempted to lock an output but the output is already
    /// a substate (Up or DOWN)
    SomeOutputSubstatesExist,
}

impl SubstateLockState {
    pub fn is_acquired(&self) -> bool {
        matches!(self, Self::LockAcquired)
    }
}
