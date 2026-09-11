//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, fmt::Display, hash::Hash};

use log::*;
use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_engine_types::substate::{Substate, SubstateId, SubstateValue};
use tari_ootle_common_types::{
    LockIntent,
    SubstateAddress,
    SubstateLockType,
    SubstateRequirementRef,
    ToSubstateAddress,
    VersionedSubstateId,
};

use crate::consensus_models::ShardGroupEvidence;

const LOG_TARGET: &str = "tari::ootle::storage::consensus_models::block_pledges";

pub type SubstatePledges = Vec<SubstatePledge>;

#[derive(Debug, Clone, Serialize, Deserialize, Default, Encode, Decode, CborLen)]
pub struct BlockPledge {
    #[n(0)]
    pledges: HashMap<SubstateId, Substate>,
}

impl BlockPledge {
    pub fn new() -> Self {
        Self {
            pledges: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.pledges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pledges.is_empty()
    }

    pub fn contains(&self, id: &SubstateId) -> bool {
        self.pledges.contains_key(id)
    }

    pub fn get_all_pledges_for_evidence(&self, evidence: &ShardGroupEvidence) -> Option<SubstatePledges> {
        let mut pledges = SubstatePledges::with_capacity(evidence.inputs().len());
        for (substate_id, ev) in evidence.all_pledged_inputs_iter() {
            // If any are missing return None
            let substate = self.pledges.get(substate_id)?;
            pledges.push(SubstatePledge::Input {
                substate_id: VersionedSubstateId::new(substate_id.clone(), substate.version()),
                is_write: ev.is_write,
                substate: Box::new(substate.substate_value().clone()),
            });
        }
        Some(pledges)
    }

    pub fn has_all_input_substate_values_for(&self, evidence: &ShardGroupEvidence) -> bool {
        if let Some((id, ev)) = evidence.all_pledged_inputs_iter().find(|(substate_id, ev)| {
            self.pledges
                .get(substate_id)
                .is_none_or(|value| value.version() != ev.version)
        }) {
            warn!(
                target: LOG_TARGET,
                "Substate not included for {} pledge: {} v{}",
                ev.as_lock_type(),
                id,
                ev.version,
            );
            return false;
        }

        true
    }

    pub fn has_some_input_substate_values_for(&self, evidence: &ShardGroupEvidence) -> bool {
        evidence.all_pledged_inputs_iter().any(|(substate_id, ev)| {
            self.pledges
                .get(substate_id)
                .is_some_and(|value| value.version() == ev.version)
        })
    }

    pub(crate) fn add_substate_pledge(
        &mut self,
        substate_id: SubstateId,
        version: u64,
        substate_value: SubstateValue,
    ) -> &mut Self {
        self.pledges.insert(substate_id, Substate::new(version, substate_value));
        self
    }
}

impl Display for BlockPledge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (id, value) in &self.pledges {
            write!(f, "{}:{},", id, value.version())?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
pub enum SubstatePledge {
    #[n(0)]
    Input {
        #[n(0)]
        substate_id: VersionedSubstateId,
        #[n(1)]
        is_write: bool,
        #[n(2)]
        substate: Box<SubstateValue>,
    },
    #[n(1)]
    Output {
        #[n(0)]
        substate_id: VersionedSubstateId,
    },
}

impl SubstatePledge {
    /// Returns a new SubstatePledge if it is valid, otherwise returns None
    /// A SubstatePledge is invalid if the lock type is either Write or Read and no substate value is provided.
    pub fn try_create<L: LockIntent>(lock_intent: L, substate: Option<SubstateValue>) -> Option<Self> {
        match lock_intent.lock_type() {
            SubstateLockType::Write | SubstateLockType::Read => Some(Self::Input {
                is_write: lock_intent.lock_type().is_write(),
                substate_id: lock_intent.to_versioned_substate_id_ref().to_owned(),
                substate: Box::new(substate?),
            }),
            SubstateLockType::Output => Some(Self::Output {
                substate_id: lock_intent.to_versioned_substate_id_ref().to_owned(),
            }),
        }
    }

    pub fn into_input(self) -> Option<(VersionedSubstateId, SubstateValue)> {
        match self {
            Self::Input {
                substate_id, substate, ..
            } => Some((substate_id, *substate)),
            _ => None,
        }
    }

    pub fn is_output(&self) -> bool {
        matches!(self, Self::Output { .. })
    }

    pub fn is_input(&self) -> bool {
        matches!(self, Self::Input { .. })
    }

    pub fn is_write(&self) -> bool {
        match self {
            Self::Input { is_write, .. } => *is_write,
            _ => false,
        }
    }

    pub fn versioned_substate_id(&self) -> &VersionedSubstateId {
        match self {
            Self::Input { substate_id, .. } => substate_id,
            Self::Output { substate_id } => substate_id,
        }
    }

    pub fn substate_id(&self) -> &SubstateId {
        self.versioned_substate_id().substate_id()
    }

    pub fn as_substate_lock_type(&self) -> SubstateLockType {
        match self {
            Self::Input { is_write, .. } => {
                if *is_write {
                    SubstateLockType::Write
                } else {
                    SubstateLockType::Read
                }
            },
            Self::Output { .. } => SubstateLockType::Output,
        }
    }

    pub fn to_substate_address(&self) -> SubstateAddress {
        self.versioned_substate_id().to_substate_address()
    }

    pub fn satisfies_requirement<'a, T: Into<SubstateRequirementRef<'a>>>(&self, req: T) -> bool {
        let req = req.into();
        // Check if a requirement is met by this pledge. If the requirement does not specify a version, then the version
        // requirement is, by definition, met.
        req.version.is_none_or(|v| v == self.versioned_substate_id().version()) &&
            self.substate_id() == req.substate_id()
    }

    pub fn satisfies_substate_and_version(&self, substate_id: &SubstateId, version: u64) -> bool {
        self.versioned_substate_id().version() == version && self.substate_id() == substate_id
    }

    pub fn satisfies_lock_intent<T: LockIntent>(&self, lock_intent: T) -> bool {
        if lock_intent.version_to_lock() != self.versioned_substate_id().version() {
            return false;
        }
        let lock_type = self.as_substate_lock_type();
        if !lock_type.allows(lock_intent.lock_type()) {
            return false;
        }

        if lock_intent.substate_id() != self.substate_id() {
            return false;
        }
        true
    }

    pub fn substate_value(&self) -> Option<&SubstateValue> {
        match self {
            Self::Input { substate, .. } => Some(substate),
            Self::Output { .. } => None,
        }
    }
}

/// Used to detect and prevent duplicate pledges without making SubstateValue implement Hash/PartialEq/Eq
impl Hash for SubstatePledge {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.substate_id().hash(state);
        self.versioned_substate_id().version().hash(state);
        self.as_substate_lock_type().hash(state);
    }
}

impl PartialEq for SubstatePledge {
    fn eq(&self, other: &Self) -> bool {
        // NB: important that PartialEq and Hash are consistent
        self.as_substate_lock_type() == other.as_substate_lock_type() &&
            self.versioned_substate_id() == other.versioned_substate_id()
    }
}

impl Eq for SubstatePledge {}

impl Display for SubstatePledge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubstatePledge::Input {
                substate_id, is_write, ..
            } => {
                if *is_write {
                    write!(f, "Write: {}", substate_id)
                } else {
                    write!(f, "Read: {}", substate_id)
                }
            },
            SubstatePledge::Output { substate_id } => write!(f, "Output: {}", substate_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use tari_engine_types::component::{Component, ComponentBody, ComponentHeader};
    use tari_template_lib::types::{ComponentAddress, EntityId};
    use tari_template_lib_types::{SubstateOwnerRule, access_rules::ComponentAccessRules};

    use super::*;

    fn create_substate_id(seed: u8) -> VersionedSubstateId {
        VersionedSubstateId::new(SubstateId::Component(ComponentAddress::from_array([seed; 32])), 0)
    }

    fn substate_value(seed: u8) -> SubstateValue {
        SubstateValue::Component(Component {
            header: ComponentHeader {
                template_address: Default::default(),
                owner_rule: SubstateOwnerRule::None,
                access_rules: ComponentAccessRules::allow_all(),
                entity_id: EntityId::from_array([seed; EntityId::LENGTH]),
            },
            body: ComponentBody::empty(),
        })
    }

    #[test]
    fn basic() {
        let mut pledge = BlockPledge::new();
        let substate1 = substate_value(1);
        let id1 = create_substate_id(1);
        let pledge = pledge.add_substate_pledge(id1.substate_id().clone(), id1.version(), substate1.clone());

        let substate2 = substate_value(2);
        let id2 = create_substate_id(2);
        let pledge = pledge.add_substate_pledge(id2.substate_id().clone(), id2.version(), substate2);

        let id3 = create_substate_id(3);

        assert_eq!(pledge.len(), 2);
        assert!(pledge.contains(id1.substate_id()));
        assert!(pledge.contains(id2.substate_id()));

        let mut evidence = ShardGroupEvidence::default();
        evidence.insert(id1.substate_id().clone(), id1.version(), SubstateLockType::Write);
        evidence.insert(id2.substate_id().clone(), id2.version(), SubstateLockType::Write);
        // Outputs are not applicable and are ignored
        evidence.insert(id3.substate_id().clone(), id3.version(), SubstateLockType::Output);

        assert!(pledge.has_all_input_substate_values_for(&evidence));

        evidence.insert(id3.substate_id().clone(), id3.version(), SubstateLockType::Write);
        assert!(!pledge.has_all_input_substate_values_for(&evidence));
    }

    #[test]
    fn partial_eq_and_hash_are_consistent() {
        let substate_value1 = substate_value(1);
        let id1 = create_substate_id(1);

        let pledge1 = SubstatePledge::Input {
            substate_id: id1.clone(),
            is_write: true,
            substate: Box::new(substate_value1.clone()),
        };

        let pledge2 = SubstatePledge::Input {
            substate_id: id1.clone(),
            is_write: true,
            substate: Box::new(substate_value1.clone()),
        };

        let pledge3 = SubstatePledge::Input {
            substate_id: id1,
            is_write: false,
            substate: Box::new(substate_value1),
        };

        use std::{
            collections::hash_map::DefaultHasher,
            hash::{Hash, Hasher},
        };

        let mut hasher1 = DefaultHasher::new();
        pledge1.hash(&mut hasher1);
        let hash1 = hasher1.finish();

        let mut hasher2 = DefaultHasher::new();
        pledge2.hash(&mut hasher2);
        let hash2 = hasher2.finish();

        let mut hasher3 = DefaultHasher::new();
        pledge3.hash(&mut hasher3);
        let hash3 = hasher3.finish();

        if pledge1 == pledge2 {
            assert_eq!(hash1, hash2);
        } else {
            assert_ne!(hash1, hash2);
        }

        if pledge1 == pledge3 {
            assert_eq!(hash1, hash3);
        } else {
            assert_ne!(hash1, hash3);
        }

        if pledge2 == pledge3 {
            assert_eq!(hash2, hash3);
        } else {
            assert_ne!(hash2, hash3);
        }
    }
}
