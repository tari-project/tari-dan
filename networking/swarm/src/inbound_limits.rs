//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    convert::Infallible,
    fmt,
    net::{IpAddr, Ipv6Addr},
    task::{Context, Poll},
};

use libp2p::{
    Multiaddr,
    PeerId,
    core::{Endpoint, multiaddr::Protocol, transport::PortUse},
    swarm::{
        ConnectionClosed,
        ConnectionDenied,
        ConnectionId,
        FromSwarm,
        ListenFailure,
        NetworkBehaviour,
        THandler,
        THandlerInEvent,
        THandlerOutEvent,
        ToSwarm,
        dummy,
    },
};

/// The inbound connections a single source may make the node hold. `None` leaves that bound off.
#[derive(Debug, Clone, Copy, Default)]
pub struct Limits {
    /// Connections from one IP source, pending or established.
    pub max_per_ip: Option<u32>,
    /// Connections from one IP source still completing their handshake.
    pub max_pending_per_ip: Option<u32>,
    /// Connections arriving through one relay still completing their handshake.
    pub max_pending_per_relay: Option<u32>,
}

/// Caps the inbound connections a single source can make the node hold.
///
/// A PeerId costs nothing to generate, so per-peer limits alone do not bound what one host can open.
/// A direct connection's source is its remote IP, with IPv6 taken per /64 because a single host is
/// routinely assigned a whole /64. A relayed connection carries no remote IP (its remote address is
/// only the PeerId the source claims), so its source is the relay it arrived through. Relays are the
/// ones this node holds reservations with, so a remote cannot mint new relayed sources.
///
/// The pending caps bound how much of the swarm-wide `max_pending_incoming` pool one source can
/// occupy. Relayed sources have no pending-plus-established cap, since every peer a relay forwards
/// would share it. Loopback connections, which only a process on this machine can open, are not
/// counted.
pub struct Behaviour {
    limits: Limits,
    counts: HashMap<Source, Counts>,
    connections: HashMap<ConnectionId, Tracked>,
}

impl Behaviour {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            counts: HashMap::new(),
            connections: HashMap::new(),
        }
    }

    fn check(&self, source: Source) -> Result<(), Exceeded> {
        let counts = self.counts.get(&source).copied().unwrap_or_default();
        let max_pending = match source {
            Source::Ip(_) => self.limits.max_pending_per_ip,
            Source::Relay(_) => self.limits.max_pending_per_relay,
        };
        if let Some(limit) = max_pending &&
            counts.pending >= limit
        {
            return Err(Exceeded {
                source,
                limit,
                kind: LimitKind::Pending,
            });
        }
        if let Source::Ip(_) = source &&
            let Some(limit) = self.limits.max_per_ip &&
            counts.pending + counts.established >= limit
        {
            return Err(Exceeded {
                source,
                limit,
                kind: LimitKind::Total,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// An IPv4 address, or an IPv6 address masked to its /64.
    Ip(IpAddr),
    Relay(PeerId),
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Ip(ip @ IpAddr::V4(_)) => write!(f, "{ip}"),
            Source::Ip(ip @ IpAddr::V6(_)) => write!(f, "{ip}/64"),
            Source::Relay(relay) => write!(f, "relay {relay}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Counts {
    pending: u32,
    established: u32,
}

#[derive(Debug, Clone, Copy)]
struct Tracked {
    source: Source,
    established: bool,
}

fn source(local_addr: &Multiaddr, remote_addr: &Multiaddr) -> Option<Source> {
    // A relayed listen address is `<relay addr>/p2p/<relay id>/p2p-circuit`. The relay client refuses
    // to listen on a circuit address without the relay's id, so a circuit always has one.
    let mut relay = None;
    for p in local_addr {
        match p {
            Protocol::P2p(peer) => relay = Some(peer),
            Protocol::P2pCircuit => return relay.map(Source::Relay),
            _ => {},
        }
    }
    remote_addr
        .iter()
        .find_map(|p| match p {
            Protocol::Ip4(ip) => Some(IpAddr::V4(ip)),
            Protocol::Ip6(ip) => Some(IpAddr::V6(ip)),
            _ => None,
        })
        .and_then(ip_source)
}

fn ip_source(ip: IpAddr) -> Option<Source> {
    match ip.to_canonical() {
        ip if ip.is_loopback() => None,
        IpAddr::V6(ip) => Some(Source::Ip(IpAddr::V6(Ipv6Addr::from_bits(
            ip.to_bits() & (u128::MAX << 64),
        )))),
        ip @ IpAddr::V4(_) => Some(Source::Ip(ip)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    Pending,
    Total,
}

#[derive(Debug, Clone, Copy)]
pub struct Exceeded {
    pub source: Source,
    pub limit: u32,
    pub kind: LimitKind,
}

impl fmt::Display for Exceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            LimitKind::Pending => write!(
                f,
                "{} already has {} inbound connections completing their handshake",
                self.source, self.limit
            ),
            LimitKind::Total => write!(f, "{} already has {} inbound connections", self.source, self.limit),
        }
    }
}

impl std::error::Error for Exceeded {}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = dummy::ConnectionHandler;
    type ToSwarm = Infallible;

    fn handle_pending_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        let Some(source) = source(local_addr, remote_addr) else {
            return Ok(());
        };
        self.check(source).map_err(ConnectionDenied::new)?;
        self.counts.entry(source).or_default().pending += 1;
        self.connections.insert(connection_id, Tracked {
            source,
            established: false,
        });
        Ok(())
    }

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        if let Some(tracked) = self.connections.get_mut(&connection_id) &&
            !tracked.established &&
            let Some(counts) = self.counts.get_mut(&tracked.source)
        {
            tracked.established = true;
            counts.pending -= 1;
            counts.established += 1;
        }
        Ok(dummy::ConnectionHandler)
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(dummy::ConnectionHandler)
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        let connection_id = match event {
            FromSwarm::ConnectionClosed(ConnectionClosed { connection_id, .. }) |
            FromSwarm::ListenFailure(ListenFailure { connection_id, .. }) => connection_id,
            _ => return,
        };
        if let Some(tracked) = self.connections.remove(&connection_id) &&
            let Some(counts) = self.counts.get_mut(&tracked.source)
        {
            if tracked.established {
                counts.established -= 1;
            } else {
                counts.pending -= 1;
            }
            if counts.pending == 0 && counts.established == 0 {
                self.counts.remove(&tracked.source);
            }
        }
    }

    fn on_connection_handler_event(&mut self, _: PeerId, _: ConnectionId, event: THandlerOutEvent<Self>) {
        match event {}
    }

    fn poll(&mut self, _: &mut Context<'_>) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELAY: &str = "12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN";

    fn addr(s: &str) -> Multiaddr {
        s.parse().unwrap()
    }

    fn direct_local() -> Multiaddr {
        addr("/ip4/0.0.0.0/tcp/1")
    }

    fn relayed_local() -> Multiaddr {
        addr(&format!("/ip4/9.9.9.9/tcp/1/p2p/{RELAY}/p2p-circuit"))
    }

    fn relayed_remote() -> Multiaddr {
        addr(&format!("/p2p/{}", PeerId::random()))
    }

    fn pending(
        limits: &mut Behaviour,
        id: usize,
        local: &Multiaddr,
        remote: &Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        limits.handle_pending_inbound_connection(ConnectionId::new_unchecked(id), local, remote)
    }

    fn establish(limits: &mut Behaviour, id: usize, remote: &Multiaddr) {
        limits
            .handle_established_inbound_connection(
                ConnectionId::new_unchecked(id),
                PeerId::random(),
                &direct_local(),
                remote,
            )
            .unwrap();
    }

    fn fail(limits: &mut Behaviour, id: usize, remote: &Multiaddr) {
        let error = libp2p::swarm::ListenError::Aborted;
        limits.on_swarm_event(FromSwarm::ListenFailure(ListenFailure {
            local_addr: &direct_local(),
            send_back_addr: remote,
            error: &error,
            connection_id: ConnectionId::new_unchecked(id),
            peer_id: None,
        }));
    }

    #[test]
    fn sources_are_bucketed_by_ip_or_relay() {
        let local = direct_local();
        assert_eq!(
            source(&local, &addr("/ip4/1.2.3.4/tcp/1")),
            Some(Source::Ip("1.2.3.4".parse().unwrap()))
        );
        assert_eq!(
            source(&local, &addr("/ip6/2001:db8:1:2:aaaa:bbbb:cccc:dddd/tcp/1")),
            Some(Source::Ip("2001:db8:1:2::".parse().unwrap()))
        );
        assert_eq!(
            source(&local, &addr("/ip6/::ffff:1.2.3.4/tcp/1")),
            Some(Source::Ip("1.2.3.4".parse().unwrap()))
        );
        assert_eq!(source(&local, &addr("/ip4/127.0.0.1/tcp/1")), None);
        assert_eq!(source(&local, &addr("/ip6/::ffff:127.0.0.1/tcp/1")), None);
        assert_eq!(
            source(&relayed_local(), &relayed_remote()),
            Some(Source::Relay(RELAY.parse().unwrap()))
        );
    }

    #[test]
    fn concurrent_dials_from_one_ip_count_before_any_is_established() {
        let mut limits = Behaviour::new(Limits {
            max_per_ip: Some(2),
            ..Limits::default()
        });
        let local = direct_local();
        let remote = addr("/ip4/1.2.3.4/tcp/1");

        for id in 0..2 {
            pending(&mut limits, id, &local, &remote).unwrap();
        }
        assert!(pending(&mut limits, 2, &local, &remote).is_err());
        pending(&mut limits, 3, &local, &addr("/ip4/5.6.7.8/tcp/1")).unwrap();
    }

    #[test]
    fn pending_cap_frees_a_slot_once_a_connection_establishes() {
        let mut limits = Behaviour::new(Limits {
            max_per_ip: Some(4),
            max_pending_per_ip: Some(2),
            ..Limits::default()
        });
        let local = direct_local();
        let remote = addr("/ip4/1.2.3.4/tcp/1");

        pending(&mut limits, 0, &local, &remote).unwrap();
        pending(&mut limits, 1, &local, &remote).unwrap();
        assert!(pending(&mut limits, 2, &local, &remote).is_err());

        establish(&mut limits, 0, &remote);
        pending(&mut limits, 3, &local, &remote).unwrap();
        assert!(pending(&mut limits, 4, &local, &remote).is_err());

        fail(&mut limits, 1, &remote);
        pending(&mut limits, 5, &local, &remote).unwrap();
        // Three established-or-pending of four, with both pending slots taken.
        assert!(pending(&mut limits, 6, &local, &remote).is_err());
    }

    #[test]
    fn ipv6_addresses_in_one_slash_64_share_a_bucket() {
        let mut limits = Behaviour::new(Limits {
            max_pending_per_ip: Some(1),
            ..Limits::default()
        });
        let local = direct_local();

        pending(&mut limits, 0, &local, &addr("/ip6/2001:db8::1/tcp/1")).unwrap();
        assert!(pending(&mut limits, 1, &local, &addr("/ip6/2001:db8::2/tcp/1")).is_err());
        pending(&mut limits, 2, &local, &addr("/ip6/2001:db8:0:1::1/tcp/1")).unwrap();
    }

    #[test]
    fn relayed_connections_are_capped_per_relay_on_pending_only() {
        let mut limits = Behaviour::new(Limits {
            max_per_ip: Some(1),
            max_pending_per_ip: Some(1),
            max_pending_per_relay: Some(2),
        });
        let local = relayed_local();
        let remotes: Vec<_> = (0..4).map(|_| relayed_remote()).collect();

        pending(&mut limits, 0, &local, &remotes[0]).unwrap();
        pending(&mut limits, 1, &local, &remotes[1]).unwrap();
        assert!(pending(&mut limits, 2, &local, &remotes[2]).is_err());

        establish(&mut limits, 0, &remotes[0]);
        establish(&mut limits, 1, &remotes[1]);
        pending(&mut limits, 3, &local, &remotes[3]).unwrap();
    }
}
