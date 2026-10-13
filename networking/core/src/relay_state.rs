//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use libp2p::{Multiaddr, PeerId, multiaddr::Protocol, swarm::ConnectionId};
use rand::seq::IteratorRandom;

/// Maximum number of relays learnt from peer identify info that are held at once.
pub(crate) const MAX_DISCOVERED_RELAYS: usize = 32;
/// Maximum number of addresses held for a single relay learnt from peer identify info.
pub(crate) const MAX_DISCOVERED_RELAY_ADDRESSES: usize = 8;

/// Tracks relay candidates and the selected relay.
///
/// Configured relays come from the node's config and are kept for the lifetime of the node. Discovered relays are
/// peers that advertise the relay protocol themselves. Their claims are unverified, so they are bounded in number and
/// addresses, held only while the peer is connected, never dialed by bootstrap, and only selected when no configured
/// relay exists.
#[derive(Debug, Clone, Default)]
pub(crate) struct RelayState {
    selected_relay: Option<RelayPeer>,
    configured_relays: HashMap<PeerId, HashSet<Multiaddr>>,
    discovered_relays: HashMap<PeerId, HashSet<Multiaddr>>,
}

impl RelayState {
    pub fn new<I: IntoIterator<Item = (PeerId, Multiaddr)>>(known_relays: I) -> Self {
        Self {
            selected_relay: None,
            configured_relays: {
                let mut hm = HashMap::<_, HashSet<_>>::new();
                for (peer, addr) in known_relays {
                    // Ensure that the /p2p/xxx protocol is present in the address
                    let addr = if addr.iter().any(|p| matches!(p, Protocol::P2p(_))) {
                        addr
                    } else {
                        addr.with(Protocol::P2p(peer))
                    };
                    hm.entry(peer).or_default().insert(addr);
                }
                hm
            },
            discovered_relays: HashMap::new(),
        }
    }

    pub fn selected_relay(&self) -> Option<&RelayPeer> {
        self.selected_relay.as_ref()
    }

    pub fn selected_relay_mut(&mut self) -> Option<&mut RelayPeer> {
        self.selected_relay.as_mut()
    }

    /// Selects `peer_id` as the relay if it is a known candidate. A selected configured relay is never displaced by a
    /// discovered one.
    pub fn set_relay_peer(&mut self, peer_id: PeerId, dialled_address: Option<Multiaddr>) -> bool {
        if let Some(selected) = self.selected_relay.as_ref() {
            if selected.peer_id == peer_id {
                return true;
            }
            if self.is_configured_relay(&selected.peer_id) && !self.is_configured_relay(&peer_id) {
                return false;
            }
        }

        if let Some(addrs) = self.addresses_of(&peer_id) {
            self.selected_relay = Some(RelayPeer {
                peer_id,
                addresses: addrs.iter().cloned().collect(),
                circuit_connection_id: None,
                remote_address: dialled_address,
            });
            return true;
        }
        false
    }

    pub fn configured_relays(&self) -> impl Iterator<Item = (&PeerId, &HashSet<Multiaddr>)> {
        self.configured_relays.iter()
    }

    pub fn num_configured_relays(&self) -> usize {
        self.configured_relays.len()
    }

    pub fn is_configured_relay(&self, peer_id: &PeerId) -> bool {
        self.configured_relays.contains_key(peer_id)
    }

    pub fn has_active_relay(&self) -> bool {
        self.selected_relay.as_ref().map(|r| r.has_circuit()).unwrap_or(false)
    }

    /// Records the addresses a peer advertises as a relay, replacing any it advertised before. Returns false if the
    /// peer is not held as a discovered relay, because it is configured, advertises no addresses or the discovered
    /// set is full.
    pub fn set_discovered_relay<I: IntoIterator<Item = Multiaddr>>(&mut self, peer: PeerId, addresses: I) -> bool {
        if self.is_configured_relay(&peer) {
            return false;
        }
        let addresses = addresses
            .into_iter()
            .take(MAX_DISCOVERED_RELAY_ADDRESSES)
            .collect::<HashSet<_>>();
        if addresses.is_empty() {
            self.discovered_relays.remove(&peer);
            return false;
        }
        if !self.discovered_relays.contains_key(&peer) && self.discovered_relays.len() >= MAX_DISCOVERED_RELAYS {
            return false;
        }
        self.discovered_relays.insert(peer, addresses);
        true
    }

    pub fn remove_discovered_relay(&mut self, peer: &PeerId) {
        self.discovered_relays.remove(peer);
    }

    pub fn clear_selected_relay(&mut self) {
        self.selected_relay = None;
    }

    pub fn select_random_relay(&mut self) {
        let candidates = if self.configured_relays.is_empty() {
            &self.discovered_relays
        } else {
            &self.configured_relays
        };
        let Some((peer, addrs)) = candidates.iter().choose(&mut rand::rng()) else {
            return;
        };
        self.selected_relay = Some(RelayPeer {
            peer_id: *peer,
            addresses: addrs.iter().cloned().collect(),
            circuit_connection_id: None,
            remote_address: None,
        });
    }

    fn addresses_of(&self, peer_id: &PeerId) -> Option<&HashSet<Multiaddr>> {
        self.configured_relays
            .get(peer_id)
            .or_else(|| self.discovered_relays.get(peer_id))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RelayPeer {
    pub peer_id: PeerId,
    pub addresses: Vec<Multiaddr>,
    pub circuit_connection_id: Option<ConnectionId>,
    pub remote_address: Option<Multiaddr>,
}

impl RelayPeer {
    pub fn has_circuit(&self) -> bool {
        self.circuit_connection_id.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(port: u16) -> Multiaddr {
        format!("/ip4/1.1.1.1/tcp/{port}").parse().unwrap()
    }

    #[test]
    fn discovered_relays_are_bounded() {
        let mut state = RelayState::new([]);
        let first = PeerId::random();
        assert!(state.set_discovered_relay(first, [addr(1)]));
        for _ in 0..1000 {
            state.set_discovered_relay(PeerId::random(), [addr(1)]);
        }
        assert_eq!(state.discovered_relays.len(), MAX_DISCOVERED_RELAYS);

        // A held relay can still refresh its addresses when the set is full
        assert!(state.set_discovered_relay(first, [addr(2)]));
        assert_eq!(state.discovered_relays[&first], HashSet::from([addr(2)]));
    }

    #[test]
    fn discovered_relay_addresses_are_bounded_and_replaced() {
        let mut state = RelayState::new([]);
        let peer = PeerId::random();
        assert!(state.set_discovered_relay(peer, (0..1000).map(addr)));
        assert_eq!(state.discovered_relays[&peer].len(), MAX_DISCOVERED_RELAY_ADDRESSES);

        assert!(state.set_discovered_relay(peer, [addr(5000)]));
        assert_eq!(state.discovered_relays[&peer], HashSet::from([addr(5000)]));

        assert!(!state.set_discovered_relay(peer, []));
        assert!(!state.discovered_relays.contains_key(&peer));
    }

    #[test]
    fn removed_discovered_relay_frees_its_slot() {
        let mut state = RelayState::new([]);
        let peers = (0..MAX_DISCOVERED_RELAYS).map(|_| PeerId::random()).collect::<Vec<_>>();
        for peer in &peers {
            assert!(state.set_discovered_relay(*peer, [addr(1)]));
        }
        let newcomer = PeerId::random();
        assert!(!state.set_discovered_relay(newcomer, [addr(1)]));

        state.remove_discovered_relay(&peers[0]);
        assert!(state.set_discovered_relay(newcomer, [addr(1)]));
    }

    #[test]
    fn discovered_relays_are_never_dialed_by_bootstrap() {
        let configured = PeerId::random();
        let mut state = RelayState::new([(configured, addr(1))]);
        state.set_discovered_relay(PeerId::random(), [addr(2)]);
        assert_eq!(state.configured_relays().map(|(p, _)| *p).collect::<Vec<_>>(), vec![
            configured
        ]);
        assert_eq!(state.num_configured_relays(), 1);
    }

    #[test]
    fn configured_relay_is_preferred() {
        let configured = PeerId::random();
        let mut state = RelayState::new([(configured, addr(1))]);
        let discovered = (0..MAX_DISCOVERED_RELAYS).map(|_| PeerId::random()).collect::<Vec<_>>();
        for peer in &discovered {
            assert!(state.set_discovered_relay(*peer, [addr(2)]));
        }
        for _ in 0..100 {
            state.select_random_relay();
            assert_eq!(state.selected_relay().unwrap().peer_id, configured);
        }

        assert!(!state.set_relay_peer(discovered[0], None));
        assert_eq!(state.selected_relay().unwrap().peer_id, configured);
    }

    #[test]
    fn configured_relay_cannot_be_overwritten_by_discovery() {
        let configured = PeerId::random();
        let mut state = RelayState::new([(configured, addr(1))]);
        assert!(!state.set_discovered_relay(configured, [addr(2)]));
        state.select_random_relay();
        assert_eq!(state.selected_relay().unwrap().addresses, vec![
            addr(1).with(Protocol::P2p(configured))
        ]);
    }

    #[test]
    fn discovered_relay_is_selected_without_configured_relays() {
        let mut state = RelayState::new([]);
        let discovered = PeerId::random();
        state.set_discovered_relay(discovered, [addr(1)]);
        state.select_random_relay();
        assert_eq!(state.selected_relay().unwrap().peer_id, discovered);
    }
}
