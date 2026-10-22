//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{self, Display, Formatter};

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use tari_engine_types::hashing::{EngineHashDomainLabel, hasher32};
use tari_jellyfish::Version;
use tari_ootle_common_types::{Epoch, SubstateAddress, ToSubstateAddress};

use crate::{SubstateTreeChange, error::StateTreeError};

/// A running hash of every substate transition a shard has committed, which the shard's state leaf commits to
/// alongside its tree root (see [`shard_state_leaf`](crate::shard_state_leaf)).
///
/// The tree root commits to the substates that are up. It says nothing about the transitions that produced them: a
/// substate created and destroyed between two roots leaves no trace in either, yet the node that committed those
/// transitions keeps records of them (its substate history and head index) that consensus reads. The accumulator
/// binds those records to the quorum-signed header, so a node that syncs a shard's history can recompute it over
/// the transitions it is streamed and check it wherever it checks the root.
///
/// A shard starts at [`TransitionAccumulator::zero`]. Every version with at least one transition extends it with
/// the epoch the version was committed at, the version, and its transitions in canonical order; a version with no
/// transitions leaves it unchanged, so a shard's accumulator is defined by its transition history alone.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
pub struct TransitionAccumulator([u8; 32]);

impl TransitionAccumulator {
    pub const LENGTH: usize = 32;

    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The accumulator of a shard that has committed no transition.
    pub const fn zero() -> Self {
        Self([0; 32])
    }

    pub const fn into_array(self) -> [u8; 32] {
        self.0
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, TransitionAccumulatorSizeError> {
        let bytes = <[u8; 32]>::try_from(bytes).map_err(|_| TransitionAccumulatorSizeError { len: bytes.len() })?;
        Ok(Self(bytes))
    }

    /// The accumulator once `changes`, the transitions of `version` committed at `epoch`, are appended.
    ///
    /// The transitions are hashed sorted by substate address, an up before a down of the same address, so the result
    /// does not depend on the order a caller lists them in. A version can both create a versioned id and destroy it
    /// again, but it holds at most one transition of each kind per address, so a second of the same kind is rejected
    /// rather than hashed into a history no block could have committed. An up transition is hashed with its value
    /// hash, the same hash its tree leaf holds.
    pub fn extend<'a, I>(&self, epoch: Epoch, version: Version, changes: I) -> Result<Self, StateTreeError>
    where I: IntoIterator<Item = &'a SubstateTreeChange> {
        let mut transitions = changes
            .into_iter()
            .map(|change| match change {
                SubstateTreeChange::Up { id, value_hash } => {
                    (id.to_substate_address(), UP_TAG, Some(value_hash.into_array()))
                },
                SubstateTreeChange::Down { id } => (id.to_substate_address(), DOWN_TAG, None),
            })
            .collect::<Vec<(SubstateAddress, u8, Option<[u8; 32]>)>>();
        if transitions.is_empty() {
            return Ok(*self);
        }
        transitions.sort_unstable_by_key(|(address, tag, _)| (*address, *tag));
        if let Some(pair) = transitions
            .windows(2)
            .find(|pair| (pair[0].0, pair[0].1) == (pair[1].0, pair[1].1))
        {
            return Err(StateTreeError::DuplicateTransition { address: pair[0].0 });
        }

        let mut hasher = hasher32(EngineHashDomainLabel::TransitionAccumulator)
            .chain(&self.0)
            .chain(&epoch.as_u64())
            .chain(&version)
            .chain(&(transitions.len() as u64));
        for (address, tag, value_hash) in transitions {
            hasher.update(&address.into_array());
            hasher.update(&tag);
            if let Some(value_hash) = value_hash {
                hasher.update(&value_hash);
            }
        }
        Ok(Self(hasher.result().into_array()))
    }
}

const UP_TAG: u8 = 0;
const DOWN_TAG: u8 = 1;

impl Display for TransitionAccumulator {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("a transition accumulator is 32 bytes, not {len}")]
pub struct TransitionAccumulatorSizeError {
    len: usize,
}

#[cfg(test)]
mod tests {
    use tari_engine_types::{SubstateVersion, substate::SubstateId};
    use tari_ootle_common_types::VersionedSubstateId;
    use tari_template_lib_types::{ComponentAddress, Hash32, ObjectKey};

    use super::*;

    fn id(seed: u8) -> VersionedSubstateId {
        VersionedSubstateId::new(
            SubstateId::Component(ComponentAddress::new(ObjectKey::from_array([seed; ObjectKey::LENGTH]))),
            SubstateVersion::ZERO,
        )
    }

    fn up(seed: u8, value: u8) -> SubstateTreeChange {
        SubstateTreeChange::Up {
            id: id(seed),
            value_hash: Hash32::from_array([value; 32]),
        }
    }

    fn down(seed: u8) -> SubstateTreeChange {
        SubstateTreeChange::Down { id: id(seed) }
    }

    #[test]
    fn the_order_transitions_are_listed_in_does_not_matter() {
        let zero = TransitionAccumulator::zero();
        let a = zero.extend(Epoch(1), 1, &[up(1, 1), up(2, 2), down(3)]).unwrap();
        let b = zero.extend(Epoch(1), 1, &[down(3), up(2, 2), up(1, 1)]).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, zero);
    }

    #[test]
    fn it_commits_to_each_transition_the_epoch_the_version_and_the_history() {
        let zero = TransitionAccumulator::zero();
        let base = zero.extend(Epoch(1), 1, &[up(1, 1)]).unwrap();
        assert_ne!(base, zero.extend(Epoch(2), 1, &[up(1, 1)]).unwrap());
        assert_ne!(base, zero.extend(Epoch(1), 2, &[up(1, 1)]).unwrap());
        assert_ne!(base, zero.extend(Epoch(1), 1, &[up(1, 2)]).unwrap());
        assert_ne!(base, zero.extend(Epoch(1), 1, &[up(2, 1)]).unwrap());
        assert_ne!(base, zero.extend(Epoch(1), 1, &[down(1)]).unwrap());
        assert_ne!(base, zero.extend(Epoch(1), 1, &[up(1, 1), up(2, 1)]).unwrap());
        assert_ne!(base, base.extend(Epoch(1), 2, &[up(1, 1)]).unwrap());
    }

    #[test]
    fn a_version_with_no_transitions_leaves_the_accumulator_unchanged() {
        let acc = TransitionAccumulator::zero().extend(Epoch(1), 1, &[up(1, 1)]).unwrap();
        assert_eq!(acc.extend(Epoch(2), 2, &[]).unwrap(), acc);
    }

    /// A block can create a versioned id and destroy it again, which the tree collapses to no leaf.
    #[test]
    fn an_up_and_a_down_of_one_address_in_one_version_are_hashed_in_a_fixed_order() {
        let zero = TransitionAccumulator::zero();
        let a = zero.extend(Epoch(1), 1, &[up(1, 1), down(1)]).unwrap();
        let b = zero.extend(Epoch(1), 1, &[down(1), up(1, 1)]).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, zero.extend(Epoch(1), 1, &[up(1, 1)]).unwrap());
        assert_ne!(a, zero.extend(Epoch(1), 1, &[down(1)]).unwrap());
    }

    #[test]
    fn two_transitions_of_one_kind_at_one_address_are_rejected() {
        for duplicated in [[up(1, 1), up(1, 2)], [down(1), down(1)]] {
            let err = TransitionAccumulator::zero()
                .extend(Epoch(1), 1, &duplicated)
                .unwrap_err();
            assert!(
                matches!(err, StateTreeError::DuplicateTransition { address } if address == id(1).to_substate_address()),
                "{err}"
            );
        }
    }

    /// A cancelling pair - a substate created and destroyed inside one proof window - leaves the tree root as it was,
    /// and is exactly what the accumulator must expose.
    #[test]
    fn a_cancelling_pair_changes_the_accumulator() {
        let zero = TransitionAccumulator::zero();
        let honest = zero
            .extend(Epoch(1), 1, &[up(1, 1)])
            .unwrap()
            .extend(Epoch(1), 2, &[up(2, 2)])
            .unwrap();
        let with_pair = zero
            .extend(Epoch(1), 1, &[up(1, 1), up(9, 9)])
            .unwrap()
            .extend(Epoch(1), 2, &[up(2, 2), down(9)])
            .unwrap();
        assert_ne!(honest, with_pair);
    }

    /// Pins the encoding. Changing it changes every state merkle root, so it must be deliberate.
    #[test]
    fn golden_vector() {
        let acc = TransitionAccumulator::zero()
            .extend(Epoch(3), 7, &[up(1, 0xaa), down(2)])
            .unwrap();
        assert_eq!(
            acc.to_string(),
            "c30d9352115ec8c6d79abdec4d0863825a8b8308dfa76188a4fb63c0c5aac7ee"
        );
    }
}
