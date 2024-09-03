//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{shard::Shard, SubstateAddress, ToSubstateAddress, VersionedSubstateId};
use tari_engine_types::substate::Substate;
use tari_state_tree::SubstateTreeChange;
use tari_transaction::TransactionId;

use crate::consensus_models::SubstateRecord;

#[derive(Debug, Clone)]
pub enum SubstateChange {
    Up {
        id: VersionedSubstateId,
        shard: Shard,
        transaction_id: TransactionId,
        substate: Substate,
    },
    Down {
        id: VersionedSubstateId,
        shard: Shard,
        transaction_id: TransactionId,
    },
}

impl SubstateChange {
    pub fn to_substate_address(&self) -> SubstateAddress {
        match self {
            SubstateChange::Up { id, .. } => id.to_substate_address(),
            SubstateChange::Down { id, .. } => id.to_substate_address(),
        }
    }

    pub fn versioned_substate_id(&self) -> &VersionedSubstateId {
        match self {
            SubstateChange::Up { id, .. } => id,
            SubstateChange::Down { id, .. } => id,
        }
    }

    pub fn substate(&self) -> Option<&Substate> {
        match self {
            SubstateChange::Up { substate, .. } => Some(substate),
            _ => None,
        }
    }

    pub fn transaction_id(&self) -> TransactionId {
        match self {
            SubstateChange::Up { transaction_id, .. } => *transaction_id,
            SubstateChange::Down { transaction_id, .. } => *transaction_id,
        }
    }

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

    pub fn up(&self) -> Option<&Substate> {
        match self {
            SubstateChange::Up { substate, .. } => Some(substate),
            _ => None,
        }
    }

    pub fn down(&self) -> Option<&VersionedSubstateId> {
        match self {
            SubstateChange::Down { id, .. } => Some(id),
            _ => None,
        }
    }

    pub fn into_up(self) -> Option<Substate> {
        match self {
            SubstateChange::Up { substate: value, .. } => Some(value),
            _ => None,
        }
    }

    pub fn as_change_string(&self) -> &'static str {
        match self {
            SubstateChange::Up { .. } => "Up",
            SubstateChange::Down { .. } => "Down",
        }
    }
}

impl From<SubstateRecord> for SubstateChange {
    fn from(value: SubstateRecord) -> Self {
        if let Some(destroyed) = value.destroyed() {
            Self::Down {
                id: value.to_versioned_substate_id(),
                shard: destroyed.by_shard,
                transaction_id: destroyed.by_transaction,
            }
        } else {
            Self::Up {
                id: value.to_versioned_substate_id(),
                shard: value.created_by_shard,
                transaction_id: value.created_by_transaction,
                substate: value.into_substate(),
            }
        }
    }
}

impl From<&SubstateChange> for SubstateTreeChange {
    fn from(value: &SubstateChange) -> Self {
        match value {
            SubstateChange::Up { id, substate, .. } => SubstateTreeChange::Up {
                id: id.substate_id().clone(),
                value_hash: substate.to_value_hash(),
            },
            SubstateChange::Down { id, .. } => SubstateTreeChange::Down {
                id: id.substate_id().clone(),
            },
        }
    }
}
