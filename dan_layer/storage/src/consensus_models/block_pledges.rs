//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    hash::Hash,
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{SubstateLockType, SubstateRequirement, VersionedSubstateId};
use tari_engine_types::substate::{SubstateId, SubstateValue};
use tari_transaction::TransactionId;

use crate::consensus_models::VersionedSubstateIdLockIntent;
#[allow(clippy::mutable_key_type)]
pub type SubstatePledges = HashSet<SubstatePledge>;
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BlockPledge {
    pledges: HashMap<TransactionId, SubstatePledges>,
}

impl BlockPledge {
    pub fn new() -> Self {
        Self {
            pledges: HashMap::new(),
        }
    }

    pub fn contains(&self, transaction_id: &TransactionId) -> bool {
        self.pledges.contains_key(transaction_id)
    }

    pub fn add_substate_pledge(&mut self, transaction_id: TransactionId, pledge: SubstatePledge) -> bool {
        self.pledges.entry(transaction_id).or_default().insert(pledge)
    }

    pub fn remove_transaction_pledges(&mut self, transaction_id: &TransactionId) -> Option<SubstatePledges> {
        self.pledges.remove(transaction_id)
    }

    pub fn num_substates_pledged(&self) -> usize {
        self.pledges.values().map(|s| s.len()).sum()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&TransactionId, &SubstatePledges)> + '_ {
        self.pledges.iter()
    }

    pub fn reserve(&mut self, additional: usize) {
        self.pledges.reserve(additional);
    }
}

impl FromIterator<(TransactionId, SubstatePledges)> for BlockPledge {
    fn from_iter<T: IntoIterator<Item = (TransactionId, SubstatePledges)>>(iter: T) -> Self {
        Self {
            pledges: iter.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubstatePledge {
    Input {
        substate_id: VersionedSubstateId,
        is_write: bool,
        substate: SubstateValue,
    },
    Output {
        substate_id: VersionedSubstateId,
    },
}

impl SubstatePledge {
    /// Returns a new SubstatePledge if it is valid, otherwise returns None
    /// A SubstatePledge is invalid if the lock type is either Write or Read and no substate value is provided.
    pub fn try_create(lock_intent: VersionedSubstateIdLockIntent, substate: Option<SubstateValue>) -> Option<Self> {
        match lock_intent.lock_type() {
            SubstateLockType::Write | SubstateLockType::Read => Some(Self::Input {
                is_write: lock_intent.lock_type().is_write(),
                substate_id: lock_intent.into_versioned_substate_id(),
                substate: substate?,
            }),
            SubstateLockType::Output => Some(Self::Output {
                substate_id: lock_intent.into_versioned_substate_id(),
            }),
        }
    }

    pub fn into_input(self) -> Option<(VersionedSubstateId, SubstateValue)> {
        match self {
            Self::Input {
                substate_id, substate, ..
            } => Some((substate_id, substate)),
            _ => None,
        }
    }

    pub fn is_output(&self) -> bool {
        matches!(self, Self::Output { .. })
    }

    pub fn is_input(&self) -> bool {
        matches!(self, Self::Input { .. })
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
}

/// These are to detect and prevent duplicates in pledging. A pledge may only
impl Hash for SubstatePledge {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.substate_id().hash(state);
    }
}

impl PartialEq for SubstatePledge {
    fn eq(&self, other: &Self) -> bool {
        self.substate_id() == other.substate_id()
    }
}

impl Eq for SubstatePledge {}

impl PartialEq<SubstateRequirement> for SubstatePledge {
    fn eq(&self, other: &SubstateRequirement) -> bool {
        // Check if a requirement is met by this pledge. If the requirement does not specify a version, then the version
        // requirement is, by definition, met.
        other
            .version
            .map_or(true, |v| v == self.versioned_substate_id().version) &&
            self.substate_id() == other.substate_id()
    }
}

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
