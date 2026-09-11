//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, fmt, fmt::Display};

use minicbor::{CborLen, Decode, Encode};
use ootle_network::Network;
use serde::{Deserialize, Serialize};
use tari_consensus_types::LeafBlock;
use tari_engine_types::{
    published_template::PublishedTemplateMetadata,
    substate::{Substate, SubstateId, SubstateValue, hash_substate},
};
use tari_ootle_common_types::{
    Epoch,
    SubstateAddress,
    SubstateRequirement,
    VersionedSubstateId,
    VersionedSubstateIdRef,
    displayable::Displayable,
    shard::Shard,
};
use tari_ootle_transaction::TransactionId;
use tari_state_tree::{SubstateTreeChange, Version};
use tari_template_lib_types::Hash32;

use crate::{
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{SubstateLock, SubstateTransition, substate_update_batch::SubstateUpdateBatch},
};

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SubstateRecord {
    #[n(0)]
    pub substate_id: SubstateId,
    #[n(1)]
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub version: u64,
    #[n(2)]
    pub substate_value: Option<SubstateValue>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[serde(with = "ootle_serde::hex")]
    #[n(3)]
    pub state_hash: Hash32,
    #[n(4)]
    pub created: SubstateCreated,
    #[n(5)]
    pub destroyed: Option<SubstateDestroyed>,
}

impl SubstateRecord {
    pub fn new<V: Into<SubstateValueOrHash>>(
        network: Network,
        substate_id: SubstateId,
        version: u64,
        value: V,
        created: SubstateCreated,
    ) -> Self {
        let value = value.into();
        Self {
            substate_id,
            version,
            state_hash: value.to_value_hash(network, version, created.at_epoch),
            substate_value: value.into_value(),
            created,
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
        SubstateRequirement::versioned(self.substate_id.clone(), self.version)
    }

    pub fn substate_id(&self) -> &SubstateId {
        &self.substate_id
    }

    pub fn substate_value(&self) -> Option<&SubstateValue> {
        self.substate_value.as_ref()
    }

    pub fn clear_substate_value(&mut self) {
        self.substate_value = None;
    }

    pub fn set_destroyed(&mut self, destroyed: SubstateDestroyed) {
        self.destroyed = Some(destroyed);
    }

    pub fn into_substate_value(self) -> Option<SubstateValue> {
        self.substate_value
    }

    pub fn into_substate(self) -> Option<Substate> {
        Some(Substate::new(self.version, self.substate_value?))
    }

    pub fn into_substate_value_or_hash(self) -> SubstateValueOrHash {
        self.substate_value
            .map(Into::into)
            .unwrap_or_else(|| self.state_hash.into())
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// Returns the shard this substate is in.
    /// WARN: you cant trust this if this is deserialized from an untrusted source, this should be validated by locally
    /// calculating which shard this substate falls.
    pub fn shard(&self) -> Shard {
        self.created.in_shard
    }

    pub fn created(&self) -> &SubstateCreated {
        &self.created
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

    pub fn state_hash(&self) -> &Hash32 {
        &self.state_hash
    }

    pub fn into_transition(self) -> SubstateTransition {
        if self.is_up() {
            SubstateTransition::Up {
                id: self.substate_id,
                version: self.version,
                substate_or_hash: self
                    .substate_value
                    .map(Into::into)
                    .unwrap_or_else(|| self.state_hash.into()),
            }
        } else {
            SubstateTransition::Down {
                id: VersionedSubstateId::new(self.substate_id, self.version),
            }
        }
    }
}

impl SubstateRecord {
    pub fn lock_all<
        'a,
        TTx: StateStoreWriteTransaction,
        I: IntoIterator<Item = (&'a SubstateId, &'a Vec<SubstateLock>)>,
    >(
        tx: &mut TTx,
        block: &LeafBlock,
        locks: I,
    ) -> Result<(), StorageError> {
        tx.substate_locks_insert_all(block, locks)
    }

    pub fn unlock_all<'a, TTx: StateStoreWriteTransaction, I: Iterator<Item = &'a TransactionId>>(
        tx: &mut TTx,
        transaction_ids: I,
    ) -> Result<(), StorageError> {
        tx.substate_locks_remove_many_for_transactions(transaction_ids)
    }

    pub fn commit_batch<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        updates: SubstateUpdateBatch,
    ) -> Result<(), StorageError> {
        tx.substates_commit_batch(updates)?;
        Ok(())
    }

    pub fn exists<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        id: VersionedSubstateIdRef<'_>,
    ) -> Result<bool, StorageError> {
        Self::any_exist(tx, Some(id))
    }

    pub fn any_exist<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = VersionedSubstateIdRef<'a>>>(
        tx: &TTx,
        substates: I,
    ) -> Result<bool, StorageError> {
        tx.substates_any_exist(substates)
    }

    pub fn get<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        address: &SubstateAddress,
    ) -> Result<SubstateRecord, StorageError> {
        tx.substates_get(address)
    }

    pub fn substate_is_up<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        shard: &SubstateAddress,
    ) -> Result<bool, StorageError> {
        // TODO: consider optimising
        let rec = tx.substates_get(shard)?;
        Ok(rec.is_up())
    }

    pub fn get_any<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = VersionedSubstateIdRef<'a>>>(
        tx: &TTx,
        requirements: I,
    ) -> Result<(Vec<SubstateRecord>, HashSet<VersionedSubstateIdRef<'a>>), StorageError> {
        let mut substate_ids = requirements.into_iter().collect::<HashSet<_>>();
        let found = tx.substates_get_any(&substate_ids)?;
        for f in &found {
            substate_ids.remove(f.substate_id());
        }

        Ok((found, substate_ids))
    }

    pub fn get_all<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = VersionedSubstateIdRef<'a>>>(
        tx: &TTx,
        requirements: I,
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        let (found, missing) = Self::get_any(tx, requirements)?;
        if !missing.is_empty() {
            return Err(StorageError::NotFound {
                item: "SubstateRecord",
                key: missing.display().to_string(),
            });
        }
        Ok(found)
    }

    pub fn get_any_max_version<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a SubstateId>>(
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

    /// Returns (version, is_up)
    pub fn get_latest_version<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        substate_id: &SubstateId,
    ) -> Result<(u64, bool), StorageError> {
        tx.substates_get_max_version_for_substate(substate_id)
    }

    pub fn get_latest<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        substate_id: &SubstateId,
    ) -> Result<SubstateRecord, StorageError> {
        let (max_version, _) = Self::get_latest_version(tx, substate_id)?;
        let rec = Self::get(tx, &SubstateAddress::from_substate_id(substate_id, max_version))?;
        Ok(rec)
    }

    pub fn prune_downed_values<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        epoch: Epoch,
    ) -> Result<usize, StorageError> {
        tx.substates_prune_downed_values(epoch)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstateCreate {
    pub substate: SubstateData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstateDestroy {
    pub substate_id: SubstateId,
    pub version: u64,
}

impl SubstateDestroy {
    pub fn to_versioned_substate_id(&self) -> VersionedSubstateId {
        VersionedSubstateId::new(self.substate_id.clone(), self.version)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SubstateCreated {
    #[n(0)]
    pub at_epoch: Epoch,
    // Note: This field not strictly necessary, since the shard can be derived from (SubstateId, Version) and
    // NumPreshards. But the cost is negligible, and it makes the metadata more self-contained.
    #[n(1)]
    pub in_shard: Shard,
    #[n(2)]
    pub at_state_version: Version,
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SubstateDestroyed {
    #[n(0)]
    pub at_epoch: Epoch,
    #[n(1)]
    pub at_state_version: Version,
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
pub enum SubstateValueOrHash {
    #[n(0)]
    Value(#[n(0)] Box<SubstateValue>),
    #[n(1)]
    Hash(#[n(0)] Hash32),
}

impl SubstateValueOrHash {
    pub fn value(&self) -> Option<&SubstateValue> {
        match self {
            SubstateValueOrHash::Value(value) => Some(value),
            SubstateValueOrHash::Hash(_) => None,
        }
    }

    pub fn into_value(self) -> Option<SubstateValue> {
        match self {
            SubstateValueOrHash::Value(value) => Some(*value),
            SubstateValueOrHash::Hash(_) => None,
        }
    }

    pub fn hash(&self) -> Option<&Hash32> {
        match self {
            SubstateValueOrHash::Value(_) => None,
            SubstateValueOrHash::Hash(hash) => Some(hash),
        }
    }

    pub fn to_value_hash(&self, network: Network, version: u64, epoch: Epoch) -> Hash32 {
        match &self {
            SubstateValueOrHash::Value(v) => hash_substate(network, v, version, epoch),
            SubstateValueOrHash::Hash(hash) => *hash,
        }
    }
}

impl From<SubstateValue> for SubstateValueOrHash {
    fn from(value: SubstateValue) -> Self {
        Self::Value(Box::new(value))
    }
}

impl From<Hash32> for SubstateValueOrHash {
    fn from(hash: Hash32) -> Self {
        Self::Hash(hash)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstateData {
    pub substate_id: SubstateId,
    pub version: u64,
    pub value: SubstateValueOrHash,
    #[serde(default)]
    pub template_metadata: Option<PublishedTemplateMetadata>,
}

impl SubstateData {
    pub fn value(&self) -> &SubstateValueOrHash {
        &self.value
    }

    pub fn as_versioned_substate_id_ref(&self) -> VersionedSubstateIdRef<'_> {
        VersionedSubstateIdRef::new(&self.substate_id, self.version)
    }

    pub fn to_value_hash(&self, network: Network, epoch: Epoch) -> Hash32 {
        self.value.to_value_hash(network, self.version, epoch)
    }

    pub fn substate_id(&self) -> &SubstateId {
        &self.substate_id
    }
}

impl From<SubstateRecord> for SubstateData {
    fn from(value: SubstateRecord) -> Self {
        Self {
            value: value
                .substate_value
                .map(Into::into)
                .unwrap_or_else(|| value.state_hash.into()),
            substate_id: value.substate_id,
            version: value.version,
            template_metadata: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubstateUpdateProof {
    Create(Box<SubstateCreate>),
    Destroy(SubstateDestroy),
}

impl SubstateUpdateProof {
    pub fn is_create(&self) -> bool {
        matches!(self, Self::Create(_))
    }

    pub fn is_destroy(&self) -> bool {
        matches!(self, Self::Destroy(_))
    }

    pub fn substate_id(&self) -> &SubstateId {
        match self {
            Self::Create(create) => &create.substate.substate_id,
            Self::Destroy(destroyed) => &destroyed.substate_id,
        }
    }

    pub fn version(&self) -> u64 {
        match self {
            Self::Create(create) => create.substate.version,
            Self::Destroy(destroyed) => destroyed.version,
        }
    }

    pub fn to_versioned_substate_id(&self) -> VersionedSubstateId {
        VersionedSubstateId::new(self.substate_id().clone(), self.version())
    }

    pub fn as_create(&self) -> Option<&SubstateCreate> {
        match self {
            Self::Create(create) => Some(create),
            _ => None,
        }
    }

    pub fn to_tree_change(&self, network: Network, epoch: Epoch) -> SubstateTreeChange {
        match self {
            Self::Create(create) => {
                let id = create.substate.as_versioned_substate_id_ref();
                SubstateTreeChange::Up {
                    id: id.to_owned(),
                    value_hash: create.substate.to_value_hash(network, epoch),
                }
            },
            Self::Destroy(destroy) => SubstateTreeChange::Down {
                id: destroy.to_versioned_substate_id(),
            },
        }
    }
}

impl From<SubstateCreate> for SubstateUpdateProof {
    fn from(value: SubstateCreate) -> Self {
        Self::Create(Box::new(value))
    }
}

impl Display for SubstateUpdateProof {
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
