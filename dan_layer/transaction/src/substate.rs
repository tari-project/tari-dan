//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Borrow, fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{shard::Shard, NumPreshards, ShardGroup, SubstateAddress};
use tari_engine_types::{serde_with, substate::SubstateId};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct SubstateRequirement {
    #[serde(with = "serde_with::string")]
    pub substate_id: SubstateId,
    pub version: Option<u32>,
}

impl SubstateRequirement {
    pub fn new(address: SubstateId, version: Option<u32>) -> Self {
        Self {
            substate_id: address,
            version,
        }
    }

    pub fn unversioned<T: Into<SubstateId>>(id: T) -> Self {
        Self {
            substate_id: id.into(),
            version: None,
        }
    }

    pub fn with_version<T: Into<SubstateId>>(id: T, version: u32) -> Self {
        Self {
            substate_id: id.into(),
            version: Some(version),
        }
    }

    pub fn substate_id(&self) -> &SubstateId {
        &self.substate_id
    }

    pub fn into_substate_id(self) -> SubstateId {
        self.substate_id
    }

    pub fn version(&self) -> Option<u32> {
        self.version
    }

    pub fn to_substate_address(&self) -> Option<SubstateAddress> {
        self.version()
            .map(|v| SubstateAddress::from_substate_id(self.substate_id(), v))
    }

    /// Calculates and returns the shard number that this SubstateAddress belongs.
    /// A shard is a fixed division of the 256-bit shard space.
    /// If the substate version is not known, None is returned.
    pub fn to_shard(&self, num_shards: NumPreshards) -> Option<Shard> {
        self.to_substate_address().map(|a| a.to_shard(num_shards))
    }

    pub fn to_shard_group(&self, num_shards: NumPreshards, num_committees: u32) -> Option<ShardGroup> {
        self.to_substate_address()
            .map(|a| a.to_shard_group(num_shards, num_committees))
    }

    pub fn to_versioned(&self) -> Option<VersionedSubstateId> {
        self.version.map(|v| VersionedSubstateId {
            substate_id: self.substate_id.clone(),
            version: v,
        })
    }

    pub fn or_zero_version(self) -> VersionedSubstateId {
        VersionedSubstateId {
            version: self.version.unwrap_or(0),
            substate_id: self.substate_id,
        }
    }
}

impl FromStr for SubstateRequirement {
    type Err = SubstateRequirementParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(':');

        // parse the substate id
        let address = parts
            .next()
            .ok_or_else(|| SubstateRequirementParseError(s.to_string()))?;
        let address = SubstateId::from_str(address).map_err(|_| SubstateRequirementParseError(s.to_string()))?;

        // parse the version (optional)
        let version = match parts.next() {
            Some(v) => {
                let parse_version = v.parse().map_err(|_| SubstateRequirementParseError(s.to_string()))?;
                Some(parse_version)
            },
            None => None,
        };

        Ok(Self {
            substate_id: address,
            version,
        })
    }
}

impl Display for SubstateRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.version {
            Some(v) => write!(f, "{}:{}", self.substate_id, v),
            None => write!(f, "{}", self.substate_id),
        }
    }
}

impl From<VersionedSubstateId> for SubstateRequirement {
    fn from(value: VersionedSubstateId) -> Self {
        Self::with_version(value.substate_id, value.version)
    }
}

impl<T: Into<SubstateId>> From<T> for SubstateRequirement {
    fn from(value: T) -> Self {
        Self::new(value.into(), None)
    }
}

impl PartialEq for SubstateRequirement {
    fn eq(&self, other: &Self) -> bool {
        self.substate_id == other.substate_id
    }
}

impl Eq for SubstateRequirement {}

// Only consider the substate id in maps. This means that duplicates found if the substate id is the same regardless of
// the version.
impl std::hash::Hash for SubstateRequirement {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.substate_id.hash(state);
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to parse substate requirement {0}")]
pub struct SubstateRequirementParseError(String);

#[derive(Debug, Clone, Hash, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct VersionedSubstateId {
    #[serde(with = "serde_with::string")]
    pub substate_id: SubstateId,
    pub version: u32,
}

impl VersionedSubstateId {
    pub fn new(substate_id: SubstateId, version: u32) -> Self {
        Self { substate_id, version }
    }

    pub fn substate_id(&self) -> &SubstateId {
        &self.substate_id
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn to_substate_address(&self) -> SubstateAddress {
        SubstateAddress::from_substate_id(self.substate_id(), self.version())
    }

    /// Calculates and returns the shard number that this SubstateAddress belongs.
    /// A shard is an equal division of the 256-bit shard space.
    pub fn to_shard(&self, num_shards: NumPreshards) -> Shard {
        self.to_substate_address().to_shard(num_shards)
    }

    pub fn to_shard_group(&self, num_shards: NumPreshards, num_committees: u32) -> ShardGroup {
        self.to_substate_address().to_shard_group(num_shards, num_committees)
    }

    pub fn to_previous_version(&self) -> Option<Self> {
        self.version
            .checked_sub(1)
            .map(|v| Self::new(self.substate_id.clone(), v))
    }

    pub fn to_next_version(&self) -> Self {
        Self::new(self.substate_id.clone(), self.version + 1)
    }
}

impl FromStr for VersionedSubstateId {
    type Err = SubstateRequirementParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(':');

        // parse the substate id
        let address = parts
            .next()
            .ok_or_else(|| SubstateRequirementParseError(s.to_string()))?;
        let address = SubstateId::from_str(address).map_err(|_| SubstateRequirementParseError(s.to_string()))?;

        // parse the version
        let version = parts
            .next()
            .ok_or_else(|| SubstateRequirementParseError(s.to_string()))
            .and_then(|v| v.parse().map_err(|_| SubstateRequirementParseError(s.to_string())))?;

        Ok(Self {
            substate_id: address,
            version,
        })
    }
}

impl Display for VersionedSubstateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.substate_id, self.version)
    }
}

impl TryFrom<SubstateRequirement> for VersionedSubstateId {
    type Error = VersionedSubstateIdError;

    fn try_from(value: SubstateRequirement) -> Result<Self, Self::Error> {
        match value.version {
            Some(v) => Ok(Self::new(value.substate_id, v)),
            None => Err(VersionedSubstateIdError::SubstateRequirementNotVersioned(
                value.substate_id,
            )),
        }
    }
}

impl Borrow<SubstateId> for VersionedSubstateId {
    fn borrow(&self) -> &SubstateId {
        &self.substate_id
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VersionedSubstateIdError {
    #[error("Substate requirement {0} is not versioned")]
    SubstateRequirementNotVersioned(SubstateId),
}

#[cfg(test)]
mod tests {
    use indexmap::IndexSet;
    use tari_template_lib::models::{ComponentAddress, ObjectKey};

    use super::*;

    #[test]
    fn it_hashes_identically_to_a_substate_id() {
        let substate_id = SubstateId::Component(ComponentAddress::new(ObjectKey::default()));
        let mut set = IndexSet::new();
        set.extend([VersionedSubstateId::new(substate_id.clone(), 0)]);
        assert!(set.contains(&substate_id));
    }
}
