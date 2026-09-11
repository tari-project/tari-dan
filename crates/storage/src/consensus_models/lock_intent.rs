//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Borrow, fmt, hash::Hash};

use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{
    LockIntent,
    SubstateAddress,
    SubstateLockType,
    SubstateRequirement,
    VersionedSubstateId,
};

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct VersionedSubstateIdLockIntent {
    #[n(0)]
    versioned_substate_id: VersionedSubstateId,
    #[n(1)]
    lock_type: SubstateLockType,
    #[n(2)]
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

    pub fn version(&self) -> u64 {
        self.versioned_substate_id.version()
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

    fn version_to_lock(&self) -> u64 {
        self.version()
    }

    fn requested_version(&self) -> Option<u64> {
        if self.require_version {
            Some(self.version())
        } else {
            None
        }
    }
}

impl LockIntent for &VersionedSubstateIdLockIntent {
    fn substate_id(&self) -> &SubstateId {
        self.versioned_substate_id.substate_id()
    }

    fn lock_type(&self) -> SubstateLockType {
        self.lock_type
    }

    fn version_to_lock(&self) -> u64 {
        self.version()
    }

    fn requested_version(&self) -> Option<u64> {
        if self.require_version {
            Some(self.version())
        } else {
            None
        }
    }
}

impl AsRef<SubstateId> for VersionedSubstateIdLockIntent {
    fn as_ref(&self) -> &SubstateId {
        self.substate_id()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RequireLockIntentRef<'a> {
    substate_id: &'a SubstateId,
    lock_type: SubstateLockType,
    version_to_lock: u64,
}

impl<'a> RequireLockIntentRef<'a> {
    pub fn new(substate_id: &'a SubstateId, version_to_lock: u64, lock: SubstateLockType) -> Self {
        Self {
            substate_id,
            lock_type: lock,
            version_to_lock,
        }
    }
}

impl<'a> From<&'a VersionedSubstateIdLockIntent> for RequireLockIntentRef<'a> {
    fn from(intent: &'a VersionedSubstateIdLockIntent) -> Self {
        RequireLockIntentRef {
            substate_id: intent.substate_id(),
            lock_type: intent.lock_type(),
            version_to_lock: intent.version_to_lock(),
        }
    }
}

impl LockIntent for RequireLockIntentRef<'_> {
    fn substate_id(&self) -> &SubstateId {
        self.substate_id
    }

    fn lock_type(&self) -> SubstateLockType {
        self.lock_type
    }

    fn version_to_lock(&self) -> u64 {
        self.version_to_lock
    }

    fn requested_version(&self) -> Option<u64> {
        Some(self.version_to_lock)
    }
}

impl fmt::Display for RequireLockIntentRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RequiredLock[{} ver: {} ({})]",
            self.substate_id, self.version_to_lock, self.lock_type
        )
    }
}

#[derive(Debug, Clone)]
pub struct SubstateRequirementLockIntent {
    substate_requirement: SubstateRequirement,
    version_to_lock: u64,
    lock_type: SubstateLockType,
}

impl SubstateRequirementLockIntent {
    pub fn new<T: Into<SubstateRequirement>>(
        substate_requirement: T,
        version_to_lock: u64,
        lock: SubstateLockType,
    ) -> Self {
        Self {
            substate_requirement: substate_requirement.into(),
            version_to_lock,
            lock_type: lock,
        }
    }

    pub fn read<T: Into<SubstateRequirement>>(substate_id: T, version_to_lock: u64) -> Self {
        Self::new(substate_id, version_to_lock, SubstateLockType::Read)
    }

    pub fn write<T: Into<SubstateRequirement>>(substate_id: T, version_to_lock: u64) -> Self {
        Self::new(substate_id, version_to_lock, SubstateLockType::Write)
    }

    pub fn output<T: Into<SubstateRequirement>>(substate_id: T, version_to_lock: u64) -> Self {
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

    pub fn version_to_lock(&self) -> u64 {
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

impl LockIntent for &SubstateRequirementLockIntent {
    fn substate_id(&self) -> &SubstateId {
        self.substate_requirement.substate_id()
    }

    fn lock_type(&self) -> SubstateLockType {
        self.lock_type
    }

    fn version_to_lock(&self) -> u64 {
        self.version_to_lock
    }

    fn requested_version(&self) -> Option<u64> {
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

    fn version_to_lock(&self) -> u64 {
        self.version_to_lock
    }

    fn requested_version(&self) -> Option<u64> {
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
        write!(
            f,
            "{} lock ver: {} ({})",
            self.substate_requirement, self.version_to_lock, self.lock_type
        )
    }
}

impl Hash for SubstateRequirementLockIntent {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // A SubstateRequirementLockIntent is uniquely identified by the SubstateRequirement
        self.substate_requirement.hash(state);
    }
}
