//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use minicbor::{CborLen, Decode, Encode};
use ootle_network::Network;
use serde::{Deserialize, Serialize};
use tari_engine_types::substate::{Substate, SubstateId};
use tari_ootle_common_types::{
    Epoch,
    SubstateAddress,
    ToSubstateAddress,
    VersionedSubstateId,
    VersionedSubstateIdRef,
    shard::Shard,
};
use tari_state_tree::SubstateTreeChange;

use crate::consensus_models::SubstateTransition;

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
pub enum SubstateChange {
    #[n(0)]
    Up {
        #[n(0)]
        id: SubstateId,
        #[n(1)]
        shard: Shard,
        #[n(2)]
        substate: Box<Substate>,
    },
    #[n(1)]
    Down {
        #[n(0)]
        id: VersionedSubstateId,
        #[n(1)]
        shard: Shard,
    },
}

impl SubstateChange {
    pub fn to_substate_address(&self) -> SubstateAddress {
        match self {
            SubstateChange::Up { id, substate, .. } => SubstateAddress::from_substate_id(id, substate.version()),
            SubstateChange::Down { id, .. } => id.to_substate_address(),
        }
    }

    pub fn versioned_substate_id(&self) -> VersionedSubstateIdRef<'_> {
        match self {
            SubstateChange::Up { id, substate, .. } => VersionedSubstateIdRef::new(id, substate.version()),
            SubstateChange::Down { id, .. } => id.as_versioned_ref(),
        }
    }

    pub fn substate_id(&self) -> &SubstateId {
        match self {
            SubstateChange::Up { id, .. } => id,
            SubstateChange::Down { id, .. } => id.substate_id(),
        }
    }

    pub fn version(&self) -> u64 {
        self.versioned_substate_id().version()
    }

    pub fn substate(&self) -> Option<&Substate> {
        match self {
            SubstateChange::Up { substate, .. } => Some(substate),
            _ => None,
        }
    }

    /// A cached shard value. This can be calculated from the substate address, but is cached here for performance.
    pub fn shard(&self) -> Shard {
        match self {
            SubstateChange::Up { shard, .. } => *shard,
            SubstateChange::Down { shard, .. } => *shard,
        }
    }

    pub fn is_down(&self) -> bool {
        matches!(self, SubstateChange::Down { .. })
    }

    pub fn is_up(&self) -> bool {
        matches!(self, SubstateChange::Up { .. })
    }

    pub fn up_substate(&self) -> Option<&Substate> {
        match self {
            SubstateChange::Up { substate, .. } => Some(substate),
            _ => None,
        }
    }

    pub fn into_up(self) -> Option<Substate> {
        match self {
            SubstateChange::Up { substate: value, .. } => Some(*value),
            _ => None,
        }
    }

    pub fn as_change_string(&self) -> &'static str {
        match self {
            SubstateChange::Up { .. } => "Up",
            SubstateChange::Down { .. } => "Down",
        }
    }

    pub fn into_transition(self) -> SubstateTransition {
        match self {
            SubstateChange::Up { id, substate, .. } => SubstateTransition::Up {
                id,
                version: substate.version(),
                substate_or_hash: substate.into_substate_value().into(),
            },
            SubstateChange::Down { id, .. } => SubstateTransition::Down { id },
        }
    }

    pub fn to_tree_change(&self, network: Network, epoch: Epoch) -> SubstateTreeChange {
        match self {
            SubstateChange::Up { id, substate, .. } => SubstateTreeChange::Up {
                id: VersionedSubstateId::new(id.clone(), substate.version()),
                value_hash: substate.to_value_hash(network, epoch),
            },
            SubstateChange::Down { id, .. } => SubstateTreeChange::Down { id: id.clone() },
        }
    }
}

impl Display for SubstateChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubstateChange::Up { id, shard, substate } => {
                write!(f, "Up: {}v{}, {}", id, substate.version(), shard)
            },
            SubstateChange::Down { id, shard } => write!(f, "Down: {}, {}", id, shard),
        }
    }
}
