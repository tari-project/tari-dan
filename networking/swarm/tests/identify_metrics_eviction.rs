//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#![cfg(feature = "metrics")]

use libp2p::{
    Multiaddr,
    core::{ConnectedPoint, Endpoint, transport::PortUse},
    identify,
    identity::Keypair,
    swarm::{ConnectionId, SwarmEvent},
};
use prometheus_client::{encoding::text::encode, registry::Registry};
use tari_swarm::{
    TariNodeBehaviourEvent,
    messaging::BytesCodec,
    metrics::{Metrics, Recorder},
};

type Event = SwarmEvent<TariNodeBehaviourEvent<BytesCodec>>;

fn identified_peers(registry: &Registry) -> Option<i64> {
    let mut text = String::new();
    encode(&mut text, registry).unwrap();
    let needle = format!(
        "libp2p_identify_remote_protocols{{protocol=\"{}\"}}",
        identify::PROTOCOL_NAME
    );
    text.lines()
        .find_map(|line| line.strip_prefix(&needle))
        .map(|rest| rest.trim().parse().unwrap())
}

fn addr() -> Multiaddr {
    "/ip4/127.0.0.1/tcp/4001".parse().unwrap()
}

fn endpoint() -> ConnectedPoint {
    ConnectedPoint::Dialer {
        address: addr(),
        role_override: Endpoint::Dialer,
        port_use: PortUse::Reuse,
    }
}

fn identify_received(keypair: &Keypair, connection_id: ConnectionId) -> Event {
    SwarmEvent::Behaviour(TariNodeBehaviourEvent::Identify(identify::Event::Received {
        connection_id,
        peer_id: keypair.public().to_peer_id(),
        info: identify::Info {
            public_key: keypair.public(),
            protocol_version: "tari/test".to_string(),
            agent_version: "test".to_string(),
            listen_addrs: vec![addr()],
            protocols: vec![identify::PROTOCOL_NAME],
            observed_addr: addr(),
            signed_peer_record: None,
        },
    }))
}

fn connection_established(keypair: &Keypair, connection_id: ConnectionId) -> Event {
    SwarmEvent::ConnectionEstablished {
        peer_id: keypair.public().to_peer_id(),
        connection_id,
        endpoint: endpoint(),
        num_established: 1.try_into().unwrap(),
        concurrent_dial_errors: None,
        established_in: Default::default(),
    }
}

fn connection_closed(keypair: &Keypair, connection_id: ConnectionId, num_established: u32) -> Event {
    SwarmEvent::ConnectionClosed {
        peer_id: keypair.public().to_peer_id(),
        connection_id,
        endpoint: endpoint(),
        num_established,
        cause: None,
    }
}

#[test]
fn identified_peer_is_released_when_its_last_connection_closes() {
    let mut registry = Registry::default();
    let metrics = Metrics::new(&mut registry);

    let peers = (0..100).map(|_| Keypair::generate_ed25519()).collect::<Vec<_>>();
    for (i, peer) in peers.iter().enumerate() {
        let connection_id = ConnectionId::new_unchecked(i);
        metrics.record(&connection_established(peer, connection_id));
        metrics.record(&identify_received(peer, connection_id));
    }
    assert_eq!(identified_peers(&registry), Some(peers.len() as i64));

    for (i, peer) in peers.iter().enumerate() {
        metrics.record(&connection_closed(peer, ConnectionId::new_unchecked(i), 0));
    }
    assert_eq!(identified_peers(&registry), None);
}

#[test]
fn identified_peer_is_kept_while_another_connection_is_open() {
    let mut registry = Registry::default();
    let metrics = Metrics::new(&mut registry);

    let peer = Keypair::generate_ed25519();
    let first = ConnectionId::new_unchecked(0);
    let second = ConnectionId::new_unchecked(1);
    metrics.record(&connection_established(&peer, first));
    metrics.record(&connection_established(&peer, second));
    metrics.record(&identify_received(&peer, first));

    metrics.record(&connection_closed(&peer, first, 1));
    assert_eq!(identified_peers(&registry), Some(1));

    metrics.record(&connection_closed(&peer, second, 0));
    assert_eq!(identified_peers(&registry), None);
}
