//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Regression test for pending-inbound slot exhaustion by stalled handshakes.
//!
//! A peer that completes the TCP connect but never drives the multistream-select
//! / noise handshake holds one of the `max_pending_incoming` slots for as long as
//! the upgrade stays pending. Without a transport upgrade timeout those slots are
//! never reclaimed, so enough stalled sockets deny all further inbound
//! connections indefinitely.
//!
//! This test drives that exact sequence with a short `connection_upgrade_timeout`:
//! it fills the pending-inbound cap with stalled sockets, confirms the next
//! inbound is refused with a connection-limit denial, then confirms the slots are
//! reclaimed once the upgrade times out so inbound is accepted again. On a build
//! without the upgrade timeout the slots are never freed and the final step is a
//! connection-limit denial instead of an acceptance, so the test fails.
//! (Loopback is exempt from the per-IP cap, so the global pending cap is the
//! limiter here.)

use std::{collections::HashSet, net::SocketAddr, time::Duration};

use libp2p::{futures::StreamExt, multiaddr::Protocol, swarm::SwarmEvent, Multiaddr};
use tari_swarm::{config::Config, create_swarm, identity::Keypair, messaging::prost::ProstCodec};
use tokio::{net::TcpStream, sync::mpsc, time::timeout};

fn multiaddr_to_socketaddr(addr: &Multiaddr) -> SocketAddr {
    let mut ip = None;
    let mut port = None;
    for p in addr.iter() {
        match p {
            Protocol::Ip4(a) => ip = Some(std::net::IpAddr::V4(a)),
            Protocol::Ip6(a) => ip = Some(std::net::IpAddr::V6(a)),
            Protocol::Tcp(p) => port = Some(p),
            _ => {}
        }
    }
    SocketAddr::new(ip.expect("ip"), port.expect("tcp port"))
}

enum Ev {
    Incoming,
    /// An `IncomingConnectionError`, carrying its debug string so a
    /// connection-limit denial can be told apart from an upgrade-timeout failure.
    Error(String),
    Established,
}

#[derive(Default)]
struct Counts {
    incoming: u32,
    established: u32,
    limit_denials: u32,
}

fn is_limit_denial(s: &str) -> bool {
    let s = s.to_lowercase();
    s.contains("exceeded") || s.contains("limit") || s.contains("denied")
}

async fn drain_for(rx: &mut mpsc::UnboundedReceiver<Ev>, dur: Duration) -> Counts {
    let mut c = Counts::default();
    let deadline = tokio::time::Instant::now() + dur;
    while let Ok(Some(ev)) = timeout(
        deadline.saturating_duration_since(tokio::time::Instant::now()),
        rx.recv(),
    )
    .await
    {
        match ev {
            Ev::Incoming => c.incoming += 1,
            Ev::Established => c.established += 1,
            Ev::Error(s) if is_limit_denial(&s) => c.limit_denials += 1,
            Ev::Error(_) => {}
        }
    }
    c
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_inbound_handshakes_are_reclaimed_after_upgrade_timeout() {
    const MAX_PENDING: u32 = 2;
    const UPGRADE_TIMEOUT: Duration = Duration::from_secs(2);

    let config = Config {
        max_pending_incoming: Some(MAX_PENDING),
        connection_upgrade_timeout: UPGRADE_TIMEOUT,
        enable_mdns: false,
        ..Default::default()
    };

    let mut swarm =
        create_swarm::<ProstCodec<Vec<u8>>>(Keypair::generate_ed25519(), HashSet::new(), config)
            .expect("create_swarm");
    swarm
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .expect("listen_on");

    let listen_addr = loop {
        if let SwarmEvent::NewListenAddr { address, .. } = swarm.select_next_some().await {
            break address;
        }
    };
    let sock = multiaddr_to_socketaddr(&listen_addr);

    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            match swarm.select_next_some().await {
                SwarmEvent::IncomingConnection { .. } => {
                    let _ = tx.send(Ev::Incoming);
                }
                SwarmEvent::IncomingConnectionError { error, .. } => {
                    let _ = tx.send(Ev::Error(format!("{error:?}")));
                }
                SwarmEvent::ConnectionEstablished { .. } => {
                    let _ = tx.send(Ev::Established);
                }
                _ => {}
            }
        }
    });

    // Fill the pending-inbound cap with sockets that connect and send nothing.
    let mut held = Vec::new();
    for _ in 0..MAX_PENDING {
        held.push(TcpStream::connect(sock).await.expect("stall connect"));
    }
    let c = drain_for(&mut rx, Duration::from_millis(800)).await;
    assert_eq!(c.established, 0, "stalled sockets must not establish");
    assert_eq!(c.limit_denials, 0, "the first {MAX_PENDING} fill the cap but are not denied");
    assert!(c.incoming >= MAX_PENDING, "listener should accept {MAX_PENDING} pending, saw {}", c.incoming);

    // Cap is full: one more inbound is refused with a connection-limit denial
    // (this is the exposure; the stalled sockets have not yet timed out).
    let extra = TcpStream::connect(sock).await.expect("extra connect");
    let c = drain_for(&mut rx, Duration::from_millis(800)).await;
    assert_eq!(c.established, 0, "extra inbound must not establish while cap is pinned");
    assert!(c.limit_denials >= 1, "extra inbound must be connection-limit denied while slots are pinned");
    drop(extra);

    // After the upgrade timeout elapses the stalled upgrades fail and free their
    // slots. Drain those (expected) timeout-failure events first.
    tokio::time::sleep(UPGRADE_TIMEOUT + Duration::from_secs(1)).await;
    let _ = drain_for(&mut rx, Duration::from_millis(300)).await;

    // A fresh inbound connection is now accepted again with no connection-limit
    // denial. Without the upgrade timeout the slots stay pinned and this is a
    // limit denial instead.
    let _recovered = TcpStream::connect(sock).await.expect("recovery connect");
    let c = drain_for(&mut rx, Duration::from_secs(2)).await;
    assert!(
        c.incoming >= 1 && c.limit_denials == 0,
        "after the upgrade timeout reclaims the pinned slots, inbound must be accepted again \
         (incoming={}, limit_denials={})",
        c.incoming,
        c.limit_denials,
    );

    drop(held);
}
