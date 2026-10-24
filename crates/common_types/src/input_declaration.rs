//    Copyright 2026 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Borrow, fmt::Display, str::FromStr};

use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use tari_engine_types::{SubstateVersion, substate::SubstateId};

use crate::{
    NumPreshards,
    SubstateAddress,
    SubstateLockType,
    SubstateRequirement,
    SubstateRequirementRef,
    VersionedSubstateId,
    shard::Shard,
};

/// One input substate a transaction declares, and the access it intends to take on it.
///
/// A shard group that does not hold an input cannot execute to discover how it is used, so it locks
/// from the declaration alone. `is_write` is what lets that lock be a read lock: read-declared
/// inputs admit concurrent readers, write-declared inputs exclude everything. Writing to an input
/// declared read aborts the transaction in the engine, so the declaration is an upper bound on
/// access rather than a hint.
#[derive(
    Debug, Clone, Deserialize, Serialize, borsh::BorshSerialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct InputDeclaration {
    #[n(0)]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub substate_id: SubstateId,
    #[n(1)]
    pub version: Option<SubstateVersion>,
    /// Whether the transaction intends to write to this input. A JSON declaration that omits it is
    /// taken as a write: an over-declaring transaction locks more than it needs and runs slowly,
    /// where an under-declaring one would abort part-way through execution. The CBOR encoding is
    /// the consensus form and requires it, so a peer cannot have its intent guessed for it.
    #[n(2)]
    #[serde(default = "write_by_default")]
    pub is_write: bool,
}

const fn write_by_default() -> bool {
    true
}

impl InputDeclaration {
    pub const fn new(substate_id: SubstateId, version: Option<SubstateVersion>, is_write: bool) -> Self {
        Self {
            substate_id,
            version,
            is_write,
        }
    }

    /// A write declaration, the conservative default for a caller that does not know.
    pub fn write<T: Into<SubstateId>>(id: T) -> Self {
        Self::new(id.into(), None, true)
    }

    pub fn write_versioned<V: Into<SubstateVersion>, T: Into<SubstateId>>(id: T, version: V) -> Self {
        Self::new(id.into(), Some(version.into()), true)
    }

    /// A read declaration. The engine aborts the transaction if it writes to this input.
    pub fn read<T: Into<SubstateId>>(id: T) -> Self {
        Self::new(id.into(), None, false)
    }

    pub fn read_versioned<V: Into<SubstateVersion>, T: Into<SubstateId>>(id: T, version: V) -> Self {
        Self::new(id.into(), Some(version.into()), false)
    }

    pub fn substate_id(&self) -> &SubstateId {
        &self.substate_id
    }

    pub fn into_substate_id(self) -> SubstateId {
        self.substate_id
    }

    pub fn version(&self) -> Option<SubstateVersion> {
        self.version
    }

    pub fn is_write(&self) -> bool {
        self.is_write
    }

    pub fn is_read(&self) -> bool {
        !self.is_write
    }

    pub fn lock_type(&self) -> SubstateLockType {
        if self.is_write {
            SubstateLockType::Write
        } else {
            SubstateLockType::Read
        }
    }

    /// The declaration covering both `self` and `other`, which name the same substate: a write if either
    /// is, pinned to `self`'s version, or to `other`'s if `self` names none.
    pub fn merge(&self, other: &InputDeclaration) -> InputDeclaration {
        InputDeclaration::new(
            self.substate_id.clone(),
            self.version.or(other.version),
            self.is_write || other.is_write,
        )
    }

    pub fn with_is_write(mut self, is_write: bool) -> Self {
        self.is_write = is_write;
        self
    }

    pub fn with_version(self, version: SubstateVersion) -> VersionedSubstateId {
        VersionedSubstateId::new(self.substate_id, version)
    }

    pub fn to_substate_requirement(&self) -> SubstateRequirement {
        SubstateRequirement::new(self.substate_id.clone(), self.version)
    }

    pub fn into_substate_requirement(self) -> SubstateRequirement {
        SubstateRequirement::new(self.substate_id, self.version)
    }

    pub fn as_ref(&self) -> InputDeclarationRef<'_> {
        InputDeclarationRef {
            substate_id: &self.substate_id,
            version: self.version,
            is_write: self.is_write,
        }
    }

    pub fn to_substate_address(&self) -> Option<SubstateAddress> {
        self.version
            .map(|v| SubstateAddress::from_substate_id(&self.substate_id, v))
    }

    pub fn to_shard(&self, num_shards: NumPreshards) -> Option<Shard> {
        self.to_substate_requirement().to_shard(num_shards)
    }
}

/// Every `SubstateId`-like value declares a write, because a caller that has not said otherwise has
/// not told us it only reads.
impl<T: Into<SubstateId>> From<T> for InputDeclaration {
    fn from(value: T) -> Self {
        Self::write(value)
    }
}

impl From<SubstateRequirement> for InputDeclaration {
    fn from(value: SubstateRequirement) -> Self {
        Self::new(value.substate_id, value.version, true)
    }
}

impl From<VersionedSubstateId> for InputDeclaration {
    fn from(value: VersionedSubstateId) -> Self {
        let version = value.version();
        Self::new(value.into_substate_id(), Some(version), true)
    }
}

impl From<InputDeclaration> for SubstateRequirement {
    fn from(value: InputDeclaration) -> Self {
        value.into_substate_requirement()
    }
}

/// Parses `<id>[:<version>|:?][:read|:write]` with the intent and version suffixes taken from the
/// right, and a `?` version parses as unversioned so that [`Display`] round-trips.
///
/// Because suffixes are split from the right, a string NFT id containing `:` whose last segment is
/// `read`, `write`, `?`, or a version number needs an explicit `:?` or `:<version>` suffix (for
/// example `nft_<resource>_str_a:7:?` or `nft_<resource>_str_x:read:?:read`).
impl FromStr for InputDeclaration {
    type Err = InputDeclarationParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || InputDeclarationParseError(s.to_string());
        let (rest, is_write) = match s.rsplit_once(':') {
            Some((prefix, "read")) => (prefix, false),
            Some((prefix, "write")) => (prefix, true),
            _ => (s, true),
        };

        let req = SubstateRequirement::from_str(rest).map_err(|_| err())?;
        Ok(Self::new(req.substate_id, req.version, is_write))
    }
}

impl Display for InputDeclaration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.version {
            Some(v) => write!(f, "{}:{}", self.substate_id, v)?,
            None => write!(f, "{}:?", self.substate_id)?,
        }
        if !self.is_write {
            write!(f, ":read")?;
        }
        Ok(())
    }
}

/// The declared intent is not part of a declaration's identity: a transaction names a substate at
/// most once, so two declarations of the same substate are the same declaration however they differ
/// on version or intent.
impl PartialEq for InputDeclaration {
    fn eq(&self, other: &Self) -> bool {
        self.substate_id == other.substate_id
    }
}

impl Eq for InputDeclaration {}

impl std::hash::Hash for InputDeclaration {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.substate_id.hash(state);
    }
}

impl Borrow<SubstateId> for InputDeclaration {
    fn borrow(&self) -> &SubstateId {
        &self.substate_id
    }
}

impl AsRef<SubstateId> for InputDeclaration {
    fn as_ref(&self) -> &SubstateId {
        &self.substate_id
    }
}

/// Adds `decl` to a transaction's declarations, merging it into an existing declaration of the same
/// substate.
///
/// A substate that a transaction both reads and writes must be write-locked, and because a set
/// keys declarations on the substate id alone, plain insertion would let declaration order decide
/// that. A merged declaration is a write if either one is, and pins the version of the existing
/// declaration, or of `decl` if the existing one names none.
pub fn declare_input(inputs: &mut IndexSet<InputDeclaration>, decl: InputDeclaration) {
    let merged = match inputs.get(decl.substate_id()) {
        Some(existing) => existing.merge(&decl),
        None => {
            inputs.insert(decl);
            return;
        },
    };
    // `replace` keeps the declaration at its original position, which the signing preimage depends on.
    inputs.replace(merged);
}

/// [`declare_input`] for each of `decls`.
pub fn declare_inputs<I: IntoIterator<Item = InputDeclaration>>(inputs: &mut IndexSet<InputDeclaration>, decls: I) {
    for decl in decls {
        declare_input(inputs, decl);
    }
}

/// Deserializes a transaction's declarations, rejecting a substate declared more than once.
///
/// A transaction's declarations are a set keyed on the substate id, so an encoding that names a
/// substate twice has no single meaning: keeping either one would silently discard the other's intent.
pub fn deserialize_input_declarations<'de, D>(deserializer: D) -> Result<IndexSet<InputDeclaration>, D::Error>
where D: serde::Deserializer<'de> {
    let decls = Vec::<InputDeclaration>::deserialize(deserializer)?;
    let mut set = IndexSet::<InputDeclaration>::with_capacity(decls.len());
    for decl in decls {
        if let Some(existing) = set.get(decl.substate_id()) {
            return Err(serde::de::Error::custom(format!(
                "input {} is declared more than once",
                existing.substate_id()
            )));
        }
        set.insert(decl);
    }
    Ok(set)
}

#[derive(Debug, Clone, Copy)]
pub struct InputDeclarationRef<'a> {
    pub substate_id: &'a SubstateId,
    pub version: Option<SubstateVersion>,
    pub is_write: bool,
}

impl<'a> InputDeclarationRef<'a> {
    pub fn new(substate_id: &'a SubstateId, version: Option<SubstateVersion>, is_write: bool) -> Self {
        Self {
            substate_id,
            version,
            is_write,
        }
    }

    pub fn read(substate_id: &'a SubstateId) -> Self {
        Self::new(substate_id, None, false)
    }

    pub fn write(substate_id: &'a SubstateId) -> Self {
        Self::new(substate_id, None, true)
    }

    pub fn substate_id(&self) -> &'a SubstateId {
        self.substate_id
    }

    pub fn version(&self) -> Option<SubstateVersion> {
        self.version
    }

    pub fn is_write(&self) -> bool {
        self.is_write
    }

    pub fn is_read(&self) -> bool {
        !self.is_write
    }

    pub fn lock_type(&self) -> SubstateLockType {
        if self.is_write {
            SubstateLockType::Write
        } else {
            SubstateLockType::Read
        }
    }

    pub fn to_owned(&self) -> InputDeclaration {
        InputDeclaration::new(self.substate_id.clone(), self.version, self.is_write)
    }

    pub fn to_substate_requirement_ref(&self) -> SubstateRequirementRef<'a> {
        SubstateRequirementRef::new(self.substate_id, self.version)
    }

    pub fn with_version(self, version: SubstateVersion) -> crate::VersionedSubstateIdRef<'a> {
        crate::VersionedSubstateIdRef::new(self.substate_id, version)
    }

    pub fn or_zero_version(self) -> crate::VersionedSubstateIdRef<'a> {
        let v = self.version.unwrap_or(SubstateVersion::ZERO);
        self.with_version(v)
    }
}

impl<'a> From<&'a InputDeclaration> for InputDeclarationRef<'a> {
    fn from(value: &'a InputDeclaration) -> Self {
        value.as_ref()
    }
}

impl<'a> From<InputDeclarationRef<'a>> for SubstateRequirementRef<'a> {
    fn from(value: InputDeclarationRef<'a>) -> Self {
        value.to_substate_requirement_ref()
    }
}

impl Borrow<SubstateId> for InputDeclarationRef<'_> {
    fn borrow(&self) -> &SubstateId {
        self.substate_id
    }
}

impl AsRef<SubstateId> for InputDeclarationRef<'_> {
    fn as_ref(&self) -> &SubstateId {
        self.substate_id
    }
}

impl PartialEq for InputDeclarationRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.substate_id == other.substate_id
    }
}

impl Eq for InputDeclarationRef<'_> {}

impl std::hash::Hash for InputDeclarationRef<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.substate_id.hash(state);
    }
}

impl Display for InputDeclarationRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.version {
            Some(v) => write!(f, "{}:{}", self.substate_id, v)?,
            None => write!(f, "{}:?", self.substate_id)?,
        }
        if !self.is_write {
            write!(f, ":read")?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to parse input declaration {0}")]
pub struct InputDeclarationParseError(String);

#[cfg(test)]
mod tests {
    use tari_template_lib_types::{ComponentAddress, NonFungibleAddress, NonFungibleId, ObjectKey, ResourceAddress};

    use super::*;

    fn component() -> SubstateId {
        SubstateId::Component(ComponentAddress::from_array([1u8; 32]))
    }

    #[test]
    fn an_undeclared_intent_is_a_write() {
        let id: InputDeclaration = component().into();
        assert!(id.is_write());
        assert!(InputDeclaration::from_str(&component().to_string()).unwrap().is_write());
    }

    #[test]
    fn it_parses_version_and_intent_in_either_combination() {
        let id = component();
        let cases = [
            (format!("{id}"), (None, true)),
            (format!("{id}:?"), (None, true)),
            (format!("{id}:7"), (Some(SubstateVersion::new(7)), true)),
            (format!("{id}:?:read"), (None, false)),
            (format!("{id}:read"), (None, false)),
            (format!("{id}:write"), (None, true)),
            (format!("{id}:7:read"), (Some(SubstateVersion::new(7)), false)),
            (format!("{id}:7:write"), (Some(SubstateVersion::new(7)), true)),
        ];
        for (s, (version, is_write)) in cases {
            let decl = InputDeclaration::from_str(&s).unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!(decl.version(), version, "{s}");
            assert_eq!(decl.is_write(), is_write, "{s}");
        }
    }

    #[test]
    fn it_rejects_an_intent_before_a_version() {
        let id = component();
        for s in [
            format!("{id}:read:7"),
            format!("{id}:nonsense"),
            format!("{id}:7:maybe"),
        ] {
            assert!(InputDeclaration::from_str(&s).is_err(), "{s} parsed");
        }
    }

    #[test]
    fn it_round_trips_through_display() {
        let nft_with_colon = SubstateId::NonFungible(NonFungibleAddress::new(
            ResourceAddress::new(ObjectKey::from_array([2u8; 32])),
            NonFungibleId::try_from_string("a:7").unwrap(),
        ));

        for id in [component(), nft_with_colon.clone()] {
            for decl in [
                InputDeclaration::read(id.clone()),
                InputDeclaration::write(id.clone()),
                InputDeclaration::read_versioned(id.clone(), SubstateVersion::new(3)),
                InputDeclaration::write_versioned(id, SubstateVersion::new(3)),
            ] {
                let parsed = InputDeclaration::from_str(&decl.to_string()).unwrap();
                assert_eq!(parsed.substate_id(), decl.substate_id());
                assert_eq!(parsed.version(), decl.version());
                assert_eq!(parsed.is_write(), decl.is_write());
            }
        }

        let explicit_write = InputDeclaration::from_str(&format!("{nft_with_colon}:?:write")).unwrap();
        assert_eq!(explicit_write.substate_id(), &nft_with_colon);
        assert_eq!(explicit_write.version(), None);
        assert!(explicit_write.is_write());

        let nft_non_numeric_colon = SubstateId::NonFungible(NonFungibleAddress::new(
            ResourceAddress::new(ObjectKey::from_array([2u8; 32])),
            NonFungibleId::try_from_string("ns:item").unwrap(),
        ));
        let bare = InputDeclaration::from_str(&nft_non_numeric_colon.to_string()).unwrap();
        assert_eq!(bare.substate_id(), &nft_non_numeric_colon);
        assert_eq!(bare.version(), None);
        assert!(bare.is_write());

        let bare_read = InputDeclaration::from_str(&format!("{nft_non_numeric_colon}:read")).unwrap();
        assert_eq!(bare_read.substate_id(), &nft_non_numeric_colon);
        assert_eq!(bare_read.version(), None);
        assert!(bare_read.is_read());
    }

    #[test]
    fn a_set_holds_one_declaration_per_substate() {
        let mut set = IndexSet::new();
        assert!(set.insert(InputDeclaration::read(component())));
        assert!(!set.insert(InputDeclaration::write(component())));
        assert!(set[0].is_read());
    }

    #[test]
    fn declaring_a_substate_twice_keeps_the_wider_intent() {
        for (first, second) in [
            (
                InputDeclaration::read(component()),
                InputDeclaration::write(component()),
            ),
            (
                InputDeclaration::write(component()),
                InputDeclaration::read(component()),
            ),
        ] {
            let mut set = IndexSet::new();
            declare_input(&mut set, first);
            declare_input(&mut set, second);
            assert_eq!(set.len(), 1);
            assert!(set[0].is_write());
        }
    }

    #[test]
    fn widening_a_declaration_keeps_a_version_either_one_pins() {
        let mut set = IndexSet::new();
        declare_inputs(&mut set, [
            InputDeclaration::read(component()),
            InputDeclaration::write_versioned(component(), SubstateVersion::new(5)),
        ]);
        assert!(set[0].is_write());
        assert_eq!(set[0].version(), Some(SubstateVersion::new(5)));

        let mut set = IndexSet::new();
        declare_inputs(&mut set, [
            InputDeclaration::write_versioned(component(), SubstateVersion::new(3)),
            InputDeclaration::read_versioned(component(), SubstateVersion::new(5)),
        ]);
        assert!(set[0].is_write());
        assert_eq!(set[0].version(), Some(SubstateVersion::new(3)));
    }

    #[test]
    fn a_substate_declared_twice_on_the_wire_is_rejected() {
        #[derive(Deserialize)]
        struct Inputs {
            #[serde(deserialize_with = "deserialize_input_declarations")]
            inputs: IndexSet<InputDeclaration>,
        }

        let id = component().to_string();
        let json = format!(
            r#"{{"inputs":[{{"substate_id":"{id}","version":null,"is_write":false}},{{"substate_id":"{id}","version":null,"is_write":true}}]}}"#
        );
        let err = serde_json::from_str::<Inputs>(&json)
            .err()
            .expect("duplicate must be rejected");
        assert!(err.to_string().contains("declared more than once"), "{err}");

        let other = SubstateId::Component(ComponentAddress::from_array([2u8; 32]));
        let json = format!(
            r#"{{"inputs":[{{"substate_id":"{id}","version":null,"is_write":false}},{{"substate_id":"{other}","version":null}}]}}"#
        );
        let decoded = serde_json::from_str::<Inputs>(&json).unwrap();
        assert_eq!(decoded.inputs.len(), 2);
        assert!(decoded.inputs[0].is_read());
        assert!(decoded.inputs[1].is_write());
    }

    #[test]
    fn declaring_a_read_twice_stays_a_read() {
        let mut set = IndexSet::new();
        declare_inputs(&mut set, [
            InputDeclaration::read(component()),
            InputDeclaration::read_versioned(component(), SubstateVersion::new(4)),
        ]);
        assert_eq!(set.len(), 1);
        assert!(set[0].is_read());
    }
}
