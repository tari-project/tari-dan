// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! The council each network launches with, and the component that holds it.

use tari_ootle_transaction::Network;
use tari_template_lib::types::{
    SubstateOwnerRule,
    crypto::RistrettoPublicKeyBytes,
    governance::{council_owner_rule, no_council_owner_rule},
};

/// The threshold and membership of a burn rate council.
///
/// A council is fixed at genesis. The component that holds it lives on the global shard, and a
/// migration that adds a substate to a live chain shifts the state root of every shard group it
/// would live on, so a network launches with the council it is going to have. Rotating it afterwards
/// is what `BurnRateGovernance::set_council` is for, and it takes the sitting council's threshold.
///
/// An empty council is a network that has decided not to govern the rate on chain: the component is
/// still instantiated, owned by nobody, and the rate comes from `ExhaustBurnRateSchedule` — the
/// release-scheduled table — instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenesisCouncil {
    pub threshold: u16,
    pub members: Vec<RistrettoPublicKeyBytes>,
}

impl GenesisCouncil {
    pub const fn none() -> Self {
        Self {
            threshold: 0,
            members: Vec::new(),
        }
    }

    pub fn is_seated(&self) -> bool {
        self.threshold > 0
    }

    /// The owner rule the governance component is created with, which is the whole of who may move
    /// the rate on the network.
    pub fn owner_rule(&self) -> SubstateOwnerRule {
        if !self.is_seated() {
            return no_council_owner_rule();
        }
        council_owner_rule(self.threshold, &self.members)
    }

    fn validate(&self) -> Result<(), GenesisCouncilError> {
        let members = self.members.len();
        if (self.threshold == 0 && members > 0) || usize::from(self.threshold) > members {
            return Err(GenesisCouncilError::Unmeetable {
                threshold: self.threshold,
                members,
            });
        }
        if let Some(member) = self
            .members
            .iter()
            .enumerate()
            .find_map(|(i, member)| self.members[..i].contains(member).then_some(*member))
        {
            return Err(GenesisCouncilError::DuplicateMember { member });
        }
        Ok(())
    }
}

/// The council `network` seats when its operator configures none.
///
/// This is the only council MainNet can seat: its genesis is part of what every node agrees on, so
/// nothing an operator configures is safe to write into it.
pub fn genesis_council(network: Network) -> GenesisCouncil {
    match network {
        Network::MainNet => GenesisCouncil {
            threshold: 1,
            members: vec![council_member(
                "9a05fef0e26b4a87d995ccb8c48582d86485c7a2c767c77c561f3db2724a8b19",
            )],
        },
        Network::StageNet | Network::NextNet | Network::Igor | Network::Esmeralda | Network::LocalNet => {
            GenesisCouncil::none()
        },
    }
}

fn council_member(hex: &str) -> RistrettoPublicKeyBytes {
    RistrettoPublicKeyBytes::from_hex(hex).expect("a release council member is a 32-byte hex public key")
}

/// The council a node writes into genesis: the one its operator `configured`, or the network's own
/// when they configured none.
///
/// Every node on a network must write the same council, or their global shards disagree from the
/// first block. On a network the operator also runs — a devnet, or a testnet before it
/// decentralises — configuration is how every node is handed the same one. MainNet refuses a
/// configured council outright.
pub fn resolve_genesis_council(
    network: Network,
    configured: Option<GenesisCouncil>,
) -> Result<GenesisCouncil, GenesisCouncilError> {
    let Some(council) = configured else {
        return Ok(genesis_council(network));
    };
    if network == Network::MainNet {
        return Err(GenesisCouncilError::ConfiguredOnMainNet);
    }
    council.validate()?;
    Ok(council)
}

#[derive(Debug, thiserror::Error)]
pub enum GenesisCouncilError {
    #[error("A genesis council cannot be configured on MainNet: its council is fixed in the release")]
    ConfiguredOnMainNet,
    #[error("A council of {members} cannot be seated at a threshold of {threshold}")]
    Unmeetable { threshold: u16, members: usize },
    #[error("The council lists member {member} more than once")]
    DuplicateMember { member: RistrettoPublicKeyBytes },
}

#[cfg(test)]
mod tests {
    use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::ByteArray};

    use super::*;

    fn all_networks() -> Vec<Network> {
        (u8::MIN..=u8::MAX)
            .filter_map(|byte| Network::try_from(byte).ok())
            .collect()
    }

    fn member(byte: u8) -> RistrettoPublicKeyBytes {
        RistrettoPublicKeyBytes::from_bytes(&[byte; 32]).unwrap()
    }

    #[test]
    fn every_networks_council_can_meet_its_own_threshold() {
        for network in all_networks() {
            let council = genesis_council(network);
            assert!(
                usize::from(council.threshold) <= council.members.len(),
                "{network} names a threshold of {} over a council of {}",
                council.threshold,
                council.members.len()
            );
        }
    }

    #[test]
    fn every_networks_council_passes_the_checks_a_configured_one_must() {
        for network in all_networks() {
            genesis_council(network)
                .validate()
                .unwrap_or_else(|e| panic!("{network}: {e}"));
        }
    }

    #[test]
    fn every_council_member_is_a_valid_public_key() {
        for network in all_networks() {
            for member in genesis_council(network).members {
                assert!(
                    RistrettoPublicKey::from_canonical_bytes(member.as_bytes()).is_ok(),
                    "{network} seats {member}, which is not a canonical Ristretto point"
                );
            }
        }
    }

    #[test]
    fn a_network_with_no_council_admits_nobody() {
        for network in all_networks() {
            let council = genesis_council(network);
            if council.is_seated() {
                continue;
            }
            assert_eq!(council.owner_rule(), SubstateOwnerRule::None, "{network}");
        }
    }

    #[test]
    fn a_configured_council_seats_on_every_network_but_mainnet() {
        let configured = GenesisCouncil {
            threshold: 1,
            members: vec![member(1)],
        };
        for network in all_networks() {
            let resolved = resolve_genesis_council(network, Some(configured.clone()));
            if network == Network::MainNet {
                assert!(matches!(resolved, Err(GenesisCouncilError::ConfiguredOnMainNet)));
            } else {
                assert_eq!(resolved.unwrap(), configured, "{network}");
            }
        }
    }

    #[test]
    fn no_configured_council_falls_back_to_the_networks_own() {
        for network in all_networks() {
            assert_eq!(
                resolve_genesis_council(network, None).unwrap(),
                genesis_council(network),
                "{network}"
            );
        }
    }

    /// `council_owner_rule` panics on a threshold the engine would reject, and genesis writes the
    /// component straight to the state store, so a configured council is refused before it gets there.
    #[test]
    fn a_configured_council_must_be_able_to_meet_its_threshold() {
        for (threshold, members) in [(0, vec![member(1)]), (2, vec![member(1)]), (1, vec![])] {
            let council = GenesisCouncil { threshold, members };
            assert!(
                matches!(
                    resolve_genesis_council(Network::LocalNet, Some(council)),
                    Err(GenesisCouncilError::Unmeetable { .. })
                ),
                "threshold {threshold}"
            );
        }
    }

    #[test]
    fn a_configured_council_must_list_each_member_once() {
        let council = GenesisCouncil {
            threshold: 2,
            members: vec![member(1), member(2), member(1)],
        };
        assert!(matches!(
            resolve_genesis_council(Network::LocalNet, Some(council)),
            Err(GenesisCouncilError::DuplicateMember { member: duplicate }) if duplicate == member(1)
        ));
    }
}
