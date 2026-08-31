// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{self, Display};

use ootle_network::Network;
use serde::{Deserialize, Serialize};

use crate::Epoch;

#[repr(u32)]
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, borsh::BorshSerialize,
)]
#[borsh(use_discriminant = true)]
pub enum ProtocolVersion {
    /// The genesis schema, which every network starts under and which anything carrying no explicit version is
    /// under.
    #[default]
    V0 = 0,
    /// Block headers commit to their own protocol version. No network schedules this version in
    /// [`Self::activations`]; scheduling it is a deliberate per-network deployment decision.
    V1 = 1,
}

impl ProtocolVersion {
    /// The schema activation schedule for `network`, ordered by activation epoch ascending. Entry at
    /// index 0 is the genesis schema, which every network starts under.
    ///
    /// Networks run at independent epochs, so an activation is scheduled per network: the epoch at
    /// which a schema goes live on esmeralda says nothing about when it goes live on igor.
    ///
    /// NB: entries here are CONSENSUS-BOUND via `hash_substate`. Never reorder or mutate an entry
    /// after it has activated on a live network — doing so changes every hash derived under it. The
    /// match is exhaustive so that a new network must state its own schedule rather than inherit one.
    const fn activations(network: Network) -> &'static [(Epoch, Self)] {
        match network {
            Network::MainNet => &[(Epoch(0), Self::V0)],
            Network::StageNet => &[(Epoch(0), Self::V0)],
            Network::NextNet => &[(Epoch(0), Self::V0)],
            Network::Igor => &[(Epoch(0), Self::V0)],
            Network::Esmeralda => &[(Epoch(0), Self::V0)],
            Network::LocalNet => &[(Epoch(0), Self::V0)],
        }
    }

    pub fn at(network: Network, epoch: Epoch) -> Self {
        Self::activations(network)
            .iter()
            .rev()
            .find(|(at, _)| *at <= epoch)
            .map(|(_, v)| *v)
            .expect("activation schedule is never empty")
    }

    pub const fn as_u32(self) -> u32 {
        self as u32
    }

    pub const fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::V0),
            1 => Some(Self::V1),
            _ => None,
        }
    }

    /// Every activation after genesis on `network`. Genesis is the schema a network starts under, so
    /// it is never something a node can have "reached" or failed to honour.
    fn scheduled_activations(network: Network) -> &'static [(Epoch, Self)] {
        Self::activations(network).get(1..).unwrap_or_default()
    }

    /// The newest activation after genesis on `network`, if its schedule has one.
    pub fn newest_scheduled_activation(network: Network) -> Option<(Epoch, Self)> {
        Self::scheduled_activations(network).last().copied()
    }

    /// The epochs at which `network` schedules an activation after genesis, in ascending order. This
    /// is what [`Self::check_activation_schedule`] compares against and returns.
    pub fn scheduled_activation_epochs(network: Network) -> Vec<Epoch> {
        Self::scheduled_activations(network).iter().map(|(at, _)| *at).collect()
    }

    /// Guards against starting a binary whose activation schedule disagrees with the one this node
    /// has already run under.
    ///
    /// `hash_substate` selects the schema from the epoch a substate was *created* at, so the set of
    /// activations at or before the epoch a node has reached determines how every substate it holds
    /// was hashed. A binary that disagrees with the node's own history about that set silently
    /// re-hashes committed state: state sync recomputes roots that no longer match the quorum-signed
    /// ones, substate proofs stop verifying, and the node diverges from the network rather than
    /// stopping. None of that surfaces as an error at the point it goes wrong, which is why it is
    /// caught here instead.
    ///
    /// `recorded` is the schedule this node last started with and `last_known_epoch` the epoch it
    /// last observed, both read from the global metadata store. Comparing whole schedules rather
    /// than only their newest entry is what catches a skipped release — a binary that adds an
    /// activation the node has already run past *behind* one that is still ahead of it. A node with
    /// no epoch history has no state to invalidate and always passes.
    ///
    /// Returns the schedule to record for the next start.
    pub fn check_activation_schedule(
        network: Network,
        recorded: &[Epoch],
        last_known_epoch: Option<Epoch>,
    ) -> Result<Vec<Epoch>, ActivationScheduleError> {
        Self::check_schedule(Self::scheduled_activations(network), recorded, last_known_epoch)
    }

    /// [`Self::check_activation_schedule`] against an explicit schedule, so the rule can be exercised
    /// for schedules other than the ones this binary is compiled with.
    fn check_schedule(
        scheduled: &[(Epoch, Self)],
        recorded: &[Epoch],
        last_known_epoch: Option<Epoch>,
    ) -> Result<Vec<Epoch>, ActivationScheduleError> {
        let to_record = || scheduled.iter().map(|(at, _)| *at).collect();

        let Some(last_known_epoch) = last_known_epoch else {
            return Ok(to_record());
        };

        // An activation this binary schedules at or before the epoch the node reached, that the node
        // never ran under: the epochs between it and now were hashed under the superseded schema.
        if let Some((activation, _)) = scheduled
            .iter()
            .rev()
            .find(|(at, _)| *at <= last_known_epoch && !recorded.contains(at))
        {
            return Err(ActivationScheduleError::Passed {
                activation: *activation,
                last_known_epoch,
            });
        }

        // The mirror image: an activation the node has already honoured that this binary drops, which
        // rolls its hashing back to a schema the network has left behind.
        if let Some(activation) = recorded
            .iter()
            .rev()
            .find(|at| **at <= last_known_epoch && !scheduled.iter().any(|(s, _)| s == *at))
        {
            return Err(ActivationScheduleError::RolledBack {
                activation: *activation,
                last_known_epoch,
            });
        }

        Ok(to_record())
    }
}

/// This binary's activation schedule disagrees with the one the node has already run under. See
/// [`ProtocolVersion::check_activation_schedule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ActivationScheduleError {
    #[error(
        "Protocol schema activates at epoch {activation}, which this node has already passed (last known epoch \
         {last_known_epoch}) without running under it. Starting would re-hash committed state and diverge from the \
         network."
    )]
    Passed { activation: Epoch, last_known_epoch: Epoch },
    #[error(
        "This binary drops the protocol schema activation at epoch {activation}, which this node has already run \
         under (last known epoch {last_known_epoch}). Starting would hash new state under a superseded schema and \
         diverge from the network."
    )]
    RolledBack { activation: Epoch, last_known_epoch: Epoch },
}

impl TryFrom<u32> for ProtocolVersion {
    type Error = UnknownProtocolVersionError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::from_u32(value).ok_or(UnknownProtocolVersionError(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Unknown protocol version {0}")]
pub struct UnknownProtocolVersionError(pub u32);

/// Encoded as its `u32` value rather than as an enum variant, so that a version this binary does not know is a
/// decode error naming the version rather than an opaque unknown-variant error, and so the encoding does not move
/// when variants are added.
impl<C> minicbor::Encode<C> for ProtocolVersion {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        _ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        e.u32(self.as_u32())?;
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for ProtocolVersion {
    fn decode(d: &mut minicbor::Decoder<'b>, _ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        let v = d.u32()?;
        Self::from_u32(v).ok_or_else(|| minicbor::decode::Error::message(UnknownProtocolVersionError(v)))
    }
}

impl<C> minicbor::CborLen<C> for ProtocolVersion {
    fn cbor_len(&self, ctx: &mut C) -> usize {
        minicbor::CborLen::cbor_len(&self.as_u32(), ctx)
    }
}

impl Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "V{}", self.as_u32())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Derived from `Network`'s byte encoding rather than listed, so a new variant is covered without
    /// anyone remembering to add it here.
    fn all_networks() -> Vec<Network> {
        (u8::MIN..=u8::MAX)
            .filter_map(|byte| Network::try_from(byte).ok())
            .collect()
    }

    #[test]
    fn every_network_starts_at_v0() {
        for network in all_networks() {
            assert_eq!(ProtocolVersion::at(network, Epoch(0)), ProtocolVersion::V0, "{network}");
        }
    }

    #[test]
    fn far_future_resolves_to_the_newest_activation() {
        for network in all_networks() {
            let (_, newest) = *ProtocolVersion::activations(network).last().unwrap();
            assert_eq!(ProtocolVersion::at(network, Epoch(u64::MAX)), newest, "{network}");
        }
    }

    #[test]
    fn genesis_is_never_a_scheduled_activation() {
        for network in all_networks() {
            assert_eq!(
                ProtocolVersion::activations(network)[0],
                (Epoch(0), ProtocolVersion::V0),
                "{network}"
            );
            assert!(
                !ProtocolVersion::scheduled_activations(network)
                    .iter()
                    .any(|(at, _)| at.is_zero()),
                "{network}"
            );
        }
    }

    #[test]
    fn monotonic_across_activations() {
        for network in all_networks() {
            let mut prev: Option<Epoch> = None;
            for (at, _) in ProtocolVersion::activations(network) {
                if let Some(p) = prev {
                    assert!(*at >= p, "{network} schedule must be sorted ascending by epoch");
                }
                prev = Some(*at);
            }
        }
    }

    mod check_schedule {
        use super::*;

        fn schedule(epochs: &[u64]) -> Vec<(Epoch, ProtocolVersion)> {
            epochs.iter().map(|e| (Epoch(*e), ProtocolVersion::V0)).collect()
        }

        fn recorded(epochs: &[u64]) -> Vec<Epoch> {
            epochs.iter().copied().map(Epoch).collect()
        }

        fn check(
            scheduled: &[u64],
            already_recorded: &[u64],
            last_known_epoch: Option<u64>,
        ) -> Result<Vec<Epoch>, ActivationScheduleError> {
            ProtocolVersion::check_schedule(
                &schedule(scheduled),
                &recorded(already_recorded),
                last_known_epoch.map(Epoch),
            )
        }

        #[test]
        fn a_node_with_no_epoch_history_has_no_state_to_invalidate() {
            assert_eq!(check(&[1000], &[], None), Ok(recorded(&[1000])));
        }

        #[test]
        fn an_activation_ahead_of_the_node_is_recorded() {
            assert_eq!(check(&[1000], &[], Some(900)), Ok(recorded(&[1000])));
        }

        #[test]
        fn an_unchanged_schedule_is_not_rechecked() {
            assert_eq!(check(&[1000], &[1000], Some(1100)), Ok(recorded(&[1000])));
        }

        #[test]
        fn an_activation_the_node_reached_without_running_under_it_is_rejected() {
            assert_eq!(
                check(&[900], &[], Some(900)),
                Err(ActivationScheduleError::Passed {
                    activation: Epoch(900),
                    last_known_epoch: Epoch(900),
                })
            );
        }

        #[test]
        fn a_skipped_release_is_rejected() {
            // The node ran a binary that scheduled nothing, straight to one scheduling 1000 and 1200,
            // at an epoch between the two. Only the newest activation is ahead of it; 1000 is not.
            assert_eq!(
                check(&[1000, 1200], &[], Some(1100)),
                Err(ActivationScheduleError::Passed {
                    activation: Epoch(1000),
                    last_known_epoch: Epoch(1100),
                })
            );
        }

        #[test]
        fn an_activation_inserted_behind_an_unchanged_newest_entry_is_rejected() {
            // The newest entry is untouched and still ahead of the node, so nothing about it is
            // suspicious; the inserted 1000 is what the node has already run past.
            assert_eq!(
                check(&[1000, 1200], &[1200], Some(1100)),
                Err(ActivationScheduleError::Passed {
                    activation: Epoch(1000),
                    last_known_epoch: Epoch(1100),
                })
            );
        }

        #[test]
        fn dropping_an_activation_the_node_has_run_under_is_rejected() {
            assert_eq!(
                check(&[], &[1000], Some(1100)),
                Err(ActivationScheduleError::RolledBack {
                    activation: Epoch(1000),
                    last_known_epoch: Epoch(1100),
                })
            );
        }

        #[test]
        fn dropping_an_activation_that_has_not_fired_is_allowed() {
            // Rescheduling a fork that has not happened yet is the supported way to abort one.
            assert_eq!(check(&[], &[1200], Some(1100)), Ok(recorded(&[])));
            assert_eq!(check(&[1300], &[1200], Some(1100)), Ok(recorded(&[1300])));
        }
    }
}
