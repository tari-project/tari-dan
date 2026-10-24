//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Borrow, fmt::Display, str::FromStr};

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use tari_engine_types::{SubstateVersion, substate::SubstateId};
use tari_template_lib_types::TransactionReceiptAddress;

use crate::{NumPreshards, ShardGroup, SubstateAddress, ToSubstateAddress, displayable::Displayable, shard::Shard};

#[derive(
    Debug, Clone, Deserialize, Serialize, borsh::BorshSerialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SubstateRequirement {
    #[n(0)]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub substate_id: SubstateId,
    #[n(1)]
    pub version: Option<SubstateVersion>,
}

impl SubstateRequirement {
    pub const fn new(address: SubstateId, version: Option<SubstateVersion>) -> Self {
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

    pub fn versioned<T: Into<SubstateId>>(id: T, version: SubstateVersion) -> Self {
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

    pub fn into_unversioned(self) -> Self {
        Self::unversioned(self.substate_id)
    }

    pub fn version(&self) -> Option<SubstateVersion> {
        self.version
    }

    pub fn with_version(self, version: SubstateVersion) -> VersionedSubstateId {
        VersionedSubstateId::new(self.substate_id, version)
    }

    pub fn to_substate_address(&self) -> Option<SubstateAddress> {
        self.version()
            .map(|v| SubstateAddress::from_substate_id(self.substate_id(), v))
    }

    pub fn to_substate_address_zero_version(&self) -> SubstateAddress {
        SubstateAddress::from_substate_id(self.substate_id(), SubstateVersion::ZERO)
    }

    /// Calculates and returns the shard number that this SubstateAddress belongs to.
    /// A shard is a fixed division of the 256-bit shard space.
    /// If the substate is global, returns `Some(Shard::global())` regardless of version.
    /// For non-global substates, returns `None` if the version is not known.
    pub fn to_shard(&self, num_shards: NumPreshards) -> Option<Shard> {
        if self.substate_id.is_global() {
            return Some(Shard::global());
        }
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
            version: self.version.unwrap_or(SubstateVersion::ZERO),
            substate_id: self.substate_id,
        }
    }

    pub fn as_ref(&self) -> SubstateRequirementRef<'_> {
        SubstateRequirementRef {
            substate_id: &self.substate_id,
            version: self.version,
        }
    }
}

/// Parses `<id>[:<version>|:?]` with the version suffix taken from the right.
///
/// Because suffixes are split from the right, a bare (unversioned) string NFT id whose last `:`
/// segment is numeric or `?` (for example `nft_<resource>_str_a:7`) is parsed as having a version
/// suffix; include an explicit `:?` or `:<version>` (as [`Display`] always emits) to disambiguate
/// string NFT ids that contain `:`.
impl FromStr for SubstateRequirement {
    type Err = SubstateRequirementParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || SubstateRequirementParseError(s.to_string());
        let (address, version) = match s.rsplit_once(':') {
            Some((addr, "?")) => (addr, None),
            Some((addr, v)) => match v.parse() {
                Ok(version) => (addr, Some(version)),
                Err(_) => (s, None),
            },
            None => (s, None),
        };
        let substate_id = SubstateId::from_str(address).map_err(|_| err())?;

        Ok(Self { substate_id, version })
    }
}

impl Display for SubstateRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.version {
            Some(v) => write!(f, "{}:{}", self.substate_id, v),
            None => write!(f, "{}:?", self.substate_id),
        }
    }
}

impl From<VersionedSubstateId> for SubstateRequirement {
    fn from(value: VersionedSubstateId) -> Self {
        Self::versioned(value.substate_id, value.version)
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

impl PartialEq<SubstateId> for SubstateRequirement {
    fn eq(&self, other: &SubstateId) -> bool {
        self.substate_id == *other
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

impl Borrow<SubstateId> for SubstateRequirement {
    fn borrow(&self) -> &SubstateId {
        &self.substate_id
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SubstateRequirementRef<'a> {
    pub substate_id: &'a SubstateId,
    pub version: Option<SubstateVersion>,
}

impl<'a> SubstateRequirementRef<'a> {
    pub fn new(substate_id: &'a SubstateId, version: Option<SubstateVersion>) -> Self {
        Self { substate_id, version }
    }

    pub fn versioned(substate_id: &'a SubstateId, version: SubstateVersion) -> Self {
        Self::new(substate_id, Some(version))
    }

    pub fn unversioned(substate_id: &'a SubstateId) -> Self {
        Self::new(substate_id, None)
    }

    pub fn to_owned(&self) -> SubstateRequirement {
        SubstateRequirement::new(self.substate_id.clone(), self.version)
    }

    pub fn with_version(self, version: SubstateVersion) -> VersionedSubstateIdRef<'a> {
        VersionedSubstateIdRef::new(self.substate_id, version)
    }

    pub fn or_zero_version(self) -> VersionedSubstateIdRef<'a> {
        let v = self.version.unwrap_or(SubstateVersion::ZERO);
        self.with_version(v)
    }

    pub fn version(&self) -> Option<SubstateVersion> {
        self.version
    }

    pub fn substate_id(&self) -> &SubstateId {
        self.substate_id
    }
}

impl<'a> From<&'a VersionedSubstateId> for SubstateRequirementRef<'a> {
    fn from(value: &'a VersionedSubstateId) -> Self {
        Self {
            substate_id: &value.substate_id,
            version: Some(value.version),
        }
    }
}

impl<'a> From<&'a SubstateRequirement> for SubstateRequirementRef<'a> {
    fn from(value: &'a SubstateRequirement) -> Self {
        Self {
            substate_id: &value.substate_id,
            version: value.version,
        }
    }
}

impl<'a> From<VersionedSubstateIdRef<'a>> for SubstateRequirementRef<'a> {
    fn from(value: VersionedSubstateIdRef<'a>) -> Self {
        Self {
            substate_id: value.substate_id,
            version: Some(value.version),
        }
    }
}

impl Borrow<SubstateId> for SubstateRequirementRef<'_> {
    fn borrow(&self) -> &SubstateId {
        self.substate_id
    }
}

impl PartialEq for SubstateRequirementRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.substate_id == other.substate_id
    }
}

impl PartialEq<SubstateId> for SubstateRequirementRef<'_> {
    fn eq(&self, other: &SubstateId) -> bool {
        self.substate_id == other
    }
}

impl Eq for SubstateRequirementRef<'_> {}

impl AsRef<SubstateId> for SubstateRequirementRef<'_> {
    fn as_ref(&self) -> &SubstateId {
        self.substate_id
    }
}

// Only consider the substate id in maps. This means that duplicates are found if the substate id is the same regardless
// of the version.
impl std::hash::Hash for SubstateRequirementRef<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.substate_id.hash(state);
    }
}

impl Display for SubstateRequirementRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.substate_id, self.version.display())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to parse substate requirement {0}")]
pub struct SubstateRequirementParseError(String);

#[derive(
    Debug,
    Clone,
    Hash,
    PartialEq,
    Eq,
    Deserialize,
    Serialize,
    BorshSerialize,
    BorshDeserialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct VersionedSubstateId {
    #[n(0)]
    substate_id: SubstateId,
    #[n(1)]
    version: SubstateVersion,
}

impl VersionedSubstateId {
    pub fn new<T: Into<SubstateId>>(substate_id: T, version: SubstateVersion) -> Self {
        Self {
            substate_id: substate_id.into(),
            version,
        }
    }

    pub fn for_tx_receipt(id: TransactionReceiptAddress) -> Self {
        Self::new(id, SubstateVersion::ZERO)
    }

    pub fn substate_id(&self) -> &SubstateId {
        &self.substate_id
    }

    pub fn into_substate_id(self) -> SubstateId {
        self.substate_id
    }

    pub fn version(&self) -> SubstateVersion {
        self.version
    }

    pub fn to_shard(&self, num_preshards: NumPreshards) -> Shard {
        if self.substate_id.is_global() {
            return Shard::global();
        }
        self.to_substate_address().to_shard(num_preshards)
    }

    pub fn to_previous_version(&self) -> Option<Self> {
        self.version.previous().map(|v| Self::new(self.substate_id.clone(), v))
    }

    pub fn to_next_version(&self) -> Self {
        Self::new(self.substate_id.clone(), self.version.next())
    }

    pub fn into_next_version(self) -> Self {
        Self::new(self.substate_id, self.version.next())
    }

    pub fn as_versioned_ref(&self) -> VersionedSubstateIdRef<'_> {
        VersionedSubstateIdRef {
            substate_id: &self.substate_id,
            version: self.version,
        }
    }

    pub fn into_unversioned_requirement(self) -> SubstateRequirement {
        SubstateRequirement::unversioned(self.substate_id)
    }

    /// The unversioned write declaration for this substate.
    pub fn into_unversioned_declaration(self) -> crate::InputDeclaration {
        crate::InputDeclaration::write(self.substate_id)
    }
}

impl ToSubstateAddress for VersionedSubstateId {
    fn to_substate_address(&self) -> SubstateAddress {
        SubstateAddress::from_substate_id(self.substate_id(), self.version())
    }
}

impl FromStr for VersionedSubstateId {
    type Err = SubstateRequirementParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || SubstateRequirementParseError(s.to_string());
        let (address, version) = s.rsplit_once(':').ok_or_else(err)?;
        let substate_id = SubstateId::from_str(address).map_err(|_| err())?;
        let version = version.parse().map_err(|_| err())?;

        Ok(Self { substate_id, version })
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

impl AsRef<SubstateId> for VersionedSubstateId {
    fn as_ref(&self) -> &SubstateId {
        &self.substate_id
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VersionedSubstateIdRef<'a> {
    pub substate_id: &'a SubstateId,
    pub version: SubstateVersion,
}

impl<'a> VersionedSubstateIdRef<'a> {
    pub fn new(substate_id: &'a SubstateId, version: SubstateVersion) -> Self {
        Self { substate_id, version }
    }

    pub fn to_shard(&self, num_preshards: NumPreshards) -> Shard {
        if self.substate_id.is_global() {
            return Shard::global();
        }
        self.to_substate_address().to_shard(num_preshards)
    }

    pub fn substate_id(&self) -> &SubstateId {
        self.substate_id
    }

    pub fn version(&self) -> SubstateVersion {
        self.version
    }

    pub fn to_owned(&self) -> VersionedSubstateId {
        VersionedSubstateId::new(self.substate_id.clone(), self.version)
    }
}

impl ToSubstateAddress for VersionedSubstateIdRef<'_> {
    fn to_substate_address(&self) -> SubstateAddress {
        SubstateAddress::from_substate_id(self.substate_id, self.version)
    }
}

impl<'a> From<&'a VersionedSubstateId> for VersionedSubstateIdRef<'a> {
    fn from(value: &'a VersionedSubstateId) -> Self {
        Self {
            substate_id: &value.substate_id,
            version: value.version,
        }
    }
}

impl Borrow<SubstateId> for VersionedSubstateIdRef<'_> {
    fn borrow(&self) -> &SubstateId {
        self.substate_id
    }
}

impl PartialEq for VersionedSubstateIdRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.substate_id == other.substate_id
    }
}

impl PartialEq<SubstateId> for VersionedSubstateIdRef<'_> {
    fn eq(&self, other: &SubstateId) -> bool {
        self.substate_id == other
    }
}

impl Eq for VersionedSubstateIdRef<'_> {}

// Only consider the substate id in maps. This means that duplicates are cheaply found if the substate id is the same
// regardless of the version.
impl std::hash::Hash for VersionedSubstateIdRef<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.substate_id.hash(state);
    }
}

impl Display for VersionedSubstateIdRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.substate_id, self.version)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VersionedSubstateIdError {
    #[error("Substate requirement {0} is not versioned")]
    SubstateRequirementNotVersioned(SubstateId),
}

#[cfg(test)]
mod tests {
    use std::hash::{DefaultHasher, Hash, Hasher};

    use indexmap::IndexSet;
    use tari_template_lib_types::{ComponentAddress, NonFungibleAddress, NonFungibleId, ObjectKey, ResourceAddress};

    use super::*;

    fn hash<T: Hash>(t: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        t.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn it_hashes_identically_to_a_substate_id() {
        let s1 = SubstateId::Component(ComponentAddress::new(ObjectKey::from_array([1; 32])));
        let ha = hash(&s1);
        let versioned = VersionedSubstateIdRef::new(&s1, SubstateVersion::new(123));
        let hb = hash(&versioned);
        let req = SubstateRequirement::versioned(s1.clone(), SubstateVersion::ZERO);
        let hc = hash(&req);
        // Ensure the a == b.&& hash(a) == hash(b) property
        assert_eq!(ha, hb);
        assert_eq!(ha, hc);
        assert_eq!(versioned, s1);
        assert_eq!(req, s1);

        let s2 = SubstateId::Component(ComponentAddress::new(ObjectKey::from_array([2; 32])));
        let h = hash(&s2);
        assert_ne!(ha, h);

        let mut set = IndexSet::new();
        set.extend([VersionedSubstateId::new(s1.clone(), SubstateVersion::ZERO)]);
        assert!(set.contains(&s1));
        let mut set = IndexSet::new();
        set.extend([SubstateRequirement::versioned(s1.clone(), SubstateVersion::ZERO)]);
        assert!(set.contains(&s1));
    }

    #[test]
    fn it_round_trips_string_nft_ids_containing_colons() {
        let nft = SubstateId::NonFungible(NonFungibleAddress::new(
            ResourceAddress::new(ObjectKey::from_array([7; 32])),
            NonFungibleId::try_from_string("a:7").unwrap(),
        ));

        for req in [
            SubstateRequirement::unversioned(nft.clone()),
            SubstateRequirement::versioned(nft.clone(), SubstateVersion::new(3)),
        ] {
            let parsed = SubstateRequirement::from_str(&req.to_string()).unwrap();
            assert_eq!(parsed.substate_id(), req.substate_id());
            assert_eq!(parsed.version(), req.version());
        }

        let versioned = VersionedSubstateId::new(nft, SubstateVersion::new(3));
        let parsed = VersionedSubstateId::from_str(&versioned.to_string()).unwrap();
        assert_eq!(parsed, versioned);
    }

    #[test]
    fn it_rejects_trailing_segments_and_missing_versions() {
        let id = SubstateId::Component(ComponentAddress::new(ObjectKey::from_array([1; 32])));

        for invalid in [format!("{id}:3:junk"), format!("{id}:junk")] {
            assert!(SubstateRequirement::from_str(&invalid).is_err(), "{invalid} parsed");
            assert!(VersionedSubstateId::from_str(&invalid).is_err(), "{invalid} parsed");
        }

        assert!(VersionedSubstateId::from_str(&id.to_string()).is_err());
        assert!(VersionedSubstateId::from_str(&format!("{id}:?")).is_err());
    }
}
