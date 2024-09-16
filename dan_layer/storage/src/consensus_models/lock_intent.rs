//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Borrow, fmt, hash::Hash};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{LockIntent, SubstateAddress, SubstateLockType, SubstateRequirement, VersionedSubstateId};
use tari_engine_types::substate::SubstateId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct VersionedSubstateIdLockIntent {
    versioned_substate_id: VersionedSubstateId,
    lock_type: SubstateLockType,
    require_version: bool,
}

impl VersionedSubstateIdLockIntent {
    pub fn new(versioned_substate_id: VersionedSubstateId, lock: SubstateLockType, require_version: bool) -> Self {
        Self {
            versioned_substate_id,
            lock_type: lock,
            require_version,
        }
    }

    pub fn from_requirement(substate_requirement: SubstateRequirement, lock: SubstateLockType) -> Self {
        let version = substate_requirement.version();
        Self::new(
            VersionedSubstateId::new(substate_requirement.into_substate_id(), version.unwrap_or(0)),
            lock,
            version.is_some(),
        )
    }

    pub fn read(versioned_substate_id: VersionedSubstateId, require_version: bool) -> Self {
        Self::new(versioned_substate_id, SubstateLockType::Read, require_version)
    }

    pub fn write(versioned_substate_id: VersionedSubstateId, require_version: bool) -> Self {
        Self::new(versioned_substate_id, SubstateLockType::Write, require_version)
    }

    pub fn output(versioned_substate_id: VersionedSubstateId) -> Self {
        // Once we lock outputs we always require the provided version
        Self::new(versioned_substate_id, SubstateLockType::Output, true)
    }

    pub fn versioned_substate_id(&self) -> &VersionedSubstateId {
        &self.versioned_substate_id
    }

    pub fn into_versioned_substate_id(self) -> VersionedSubstateId {
        self.versioned_substate_id
    }

    pub fn substate_id(&self) -> &SubstateId {
        self.versioned_substate_id.substate_id()
    }

    pub fn version(&self) -> u32 {
        self.versioned_substate_id.version()
    }

    pub fn lock_type(&self) -> SubstateLockType {
        self.lock_type
    }

    pub fn to_substate_requirement(&self) -> SubstateRequirement {
        let version = if self.require_version {
            Some(self.version())
        } else {
            None
        };
        SubstateRequirement::new(self.substate_id().clone(), version)
    }
}

impl Borrow<SubstateId> for VersionedSubstateIdLockIntent {
    fn borrow(&self) -> &SubstateId {
        self.substate_id()
    }
}

impl fmt::Display for VersionedSubstateIdLockIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.versioned_substate_id, self.lock_type)
    }
}

impl Hash for VersionedSubstateIdLockIntent {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // A VersionedSubstateIdLockIntent is uniquely identified by the VersionedSubstateId
        self.versioned_substate_id.hash(state);
    }
}

impl LockIntent for VersionedSubstateIdLockIntent {
    fn substate_id(&self) -> &SubstateId {
        self.versioned_substate_id.substate_id()
    }

    fn lock_type(&self) -> SubstateLockType {
        self.lock_type
    }

    fn version_to_lock(&self) -> u32 {
        self.version()
    }

    fn requested_version(&self) -> Option<u32> {
        if self.require_version {
            Some(self.version())
        } else {
            None
        }
    }
}

impl<'a> LockIntent for &'a VersionedSubstateIdLockIntent {
    fn substate_id(&self) -> &SubstateId {
        self.versioned_substate_id.substate_id()
    }

    fn lock_type(&self) -> SubstateLockType {
        self.lock_type
    }

    fn version_to_lock(&self) -> u32 {
        self.version()
    }

    fn requested_version(&self) -> Option<u32> {
        if self.require_version {
            Some(self.version())
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct SubstateRequirementLockIntent {
    substate_requirement: SubstateRequirement,
    version_to_lock: u32,
    lock_type: SubstateLockType,
}

impl SubstateRequirementLockIntent {
    pub fn new<T: Into<SubstateRequirement>>(substate_id: T, version_to_lock: u32, lock: SubstateLockType) -> Self {
        Self {
            substate_requirement: substate_id.into(),
            version_to_lock,
            lock_type: lock,
        }
    }

    pub fn read<T: Into<SubstateRequirement>>(substate_id: T, version_to_lock: u32) -> Self {
        Self::new(substate_id, version_to_lock, SubstateLockType::Read)
    }

    pub fn write<T: Into<SubstateRequirement>>(substate_id: T, version_to_lock: u32) -> Self {
        Self::new(substate_id, version_to_lock, SubstateLockType::Write)
    }

    pub fn output<T: Into<SubstateRequirement>>(substate_id: T, version_to_lock: u32) -> Self {
        Self::new(substate_id, version_to_lock, SubstateLockType::Output)
    }

    pub fn to_substate_address(&self) -> Option<SubstateAddress> {
        self.substate_requirement.to_substate_address()
    }

    pub fn substate_requirement(&self) -> &SubstateRequirement {
        &self.substate_requirement
    }

    pub fn into_substate_requirement(self) -> SubstateRequirement {
        self.substate_requirement
    }

    pub fn substate_id(&self) -> &SubstateId {
        self.substate_requirement.substate_id()
    }

    pub fn version_to_lock(&self) -> u32 {
        self.version_to_lock
    }

    pub fn lock_type(&self) -> SubstateLockType {
        self.lock_type
    }

    pub fn to_versioned_lock_intent(&self) -> VersionedSubstateIdLockIntent {
        VersionedSubstateIdLockIntent::new(
            VersionedSubstateId::new(self.substate_id().clone(), self.version_to_lock),
            self.lock_type,
            self.substate_requirement.version().is_some(),
        )
    }
}

impl<'a> LockIntent for &'a SubstateRequirementLockIntent {
    fn substate_id(&self) -> &SubstateId {
        self.substate_requirement.substate_id()
    }

    fn lock_type(&self) -> SubstateLockType {
        self.lock_type
    }

    fn version_to_lock(&self) -> u32 {
        self.version_to_lock
    }

    fn requested_version(&self) -> Option<u32> {
        self.substate_requirement.version()
    }
}

impl LockIntent for SubstateRequirementLockIntent {
    fn substate_id(&self) -> &SubstateId {
        self.substate_requirement.substate_id()
    }

    fn lock_type(&self) -> SubstateLockType {
        self.lock_type
    }

    fn version_to_lock(&self) -> u32 {
        self.version_to_lock
    }

    fn requested_version(&self) -> Option<u32> {
        self.substate_requirement.version()
    }
}

impl From<VersionedSubstateIdLockIntent> for SubstateRequirementLockIntent {
    fn from(intent: VersionedSubstateIdLockIntent) -> Self {
        let version = intent.versioned_substate_id.version();
        Self::new(intent.to_substate_requirement(), version, intent.lock_type)
    }
}

impl Borrow<SubstateId> for SubstateRequirementLockIntent {
    fn borrow(&self) -> &SubstateId {
        self.substate_id()
    }
}

impl fmt::Display for SubstateRequirementLockIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.substate_requirement, self.lock_type)
    }
}

impl Hash for SubstateRequirementLockIntent {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // A SubstateRequirementLockIntent is uniquely identified by the SubstateRequirement
        self.substate_requirement.hash(state);
    }
}
