//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use indexmap::IndexMap;
use ootle_network::Network;
use tari_engine_types::{SubstateVersion, substate::SubstateId};
use tari_ootle_common_types::{Epoch, VersionedSubstateId, shard::Shard};
use tari_state_tree::{SubstateTreeChange, Version};

use crate::consensus_models::SubstateValueOrHash;

pub struct SubstateUpdateBatch {
    pub network: Network,
    pub epoch: Epoch,
    pub updates: IndexMap<Shard, IndexMap<Version, Vec<SubstateTransition>>>,
}

impl SubstateUpdateBatch {
    pub fn new(network: Network, epoch: Epoch) -> Self {
        Self {
            network,
            epoch,
            updates: IndexMap::new(),
        }
    }

    pub fn with_transition(&mut self, shard: Shard, state_version: Version) -> &mut Vec<SubstateTransition> {
        self.updates.entry(shard).or_default().entry(state_version).or_default()
    }

    /// Every substate the batch takes down.
    pub fn downed_iter(&self) -> impl Iterator<Item = DownedSubstate> + '_ {
        self.updates.iter().flat_map(move |(shard, versions)| {
            versions.iter().flat_map(move |(state_version, transitions)| {
                transitions
                    .iter()
                    .filter_map(SubstateTransition::down)
                    .map(move |id| DownedSubstate {
                        epoch: self.epoch,
                        shard: *shard,
                        id: id.clone(),
                        state_version: *state_version,
                    })
            })
        })
    }
}

/// A substate taken down at `state_version` of `shard`, in `epoch`.
#[derive(Debug, Clone)]
pub struct DownedSubstate {
    pub epoch: Epoch,
    pub shard: Shard,
    pub id: VersionedSubstateId,
    pub state_version: Version,
}

pub enum SubstateTransition {
    Up {
        id: SubstateId,
        version: SubstateVersion,
        substate_or_hash: SubstateValueOrHash,
    },
    Down {
        id: VersionedSubstateId,
    },
}

impl SubstateTransition {
    /// The change this transition makes to the shard's state tree: an up with the value hash its leaf holds, which
    /// binds `network` and the `epoch` it is committed at, or a down.
    pub fn to_tree_change(&self, network: Network, epoch: Epoch) -> SubstateTreeChange {
        match self {
            Self::Up {
                id,
                version,
                substate_or_hash,
            } => SubstateTreeChange::Up {
                id: VersionedSubstateId::new(id.clone(), *version),
                value_hash: substate_or_hash.to_value_hash(network, *version, epoch),
            },
            Self::Down { id } => SubstateTreeChange::Down { id: id.clone() },
        }
    }

    pub fn down(&self) -> Option<&VersionedSubstateId> {
        match self {
            Self::Down { id } => Some(id),
            Self::Up { .. } => None,
        }
    }
}
