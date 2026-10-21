//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub use libp2p::metrics::Recorder;
use libp2p::{rendezvous, swarm::SwarmEvent};
use prometheus_client::metrics::counter::Counter;

use crate::TariNodeBehaviourEvent;

pub struct Metrics {
    libp2p_metrics: libp2p::metrics::Metrics,
    extended_metrics: ExtendedMetrics,
}

impl Metrics {
    pub fn new(registry: &mut libp2p::metrics::Registry) -> Self {
        let libp2p_metrics = libp2p::metrics::Metrics::new(registry);
        let extended_metrics = ExtendedMetrics::new(registry);
        Self {
            libp2p_metrics,
            extended_metrics,
        }
    }
}

impl<TCodec: libp2p_messaging::Codec + Send + Clone + 'static> Recorder<TariNodeBehaviourEvent<TCodec>> for Metrics {
    fn record(&self, event: &TariNodeBehaviourEvent<TCodec>) {
        match event {
            TariNodeBehaviourEvent::Ping(ping_event) => self.libp2p_metrics.record(ping_event),
            TariNodeBehaviourEvent::Dcutr(dcutr_event) => self.libp2p_metrics.record(dcutr_event),
            TariNodeBehaviourEvent::Identify(identify_event) => self.libp2p_metrics.record(identify_event),
            TariNodeBehaviourEvent::RelayClient(_) => {}, // Metrics not implemented for RelayClient
            TariNodeBehaviourEvent::Relay(relay_event) => self.libp2p_metrics.record(relay_event),
            TariNodeBehaviourEvent::RendezvousServer(event) => self.extended_metrics.record(event),
            TariNodeBehaviourEvent::RendezvousClient(event) => self.extended_metrics.record(event),
            TariNodeBehaviourEvent::PeerStore(_) => {}, // No metrics for PeerStore events
            TariNodeBehaviourEvent::Gossipsub(gossipsub_event) => self.libp2p_metrics.record(gossipsub_event),
            TariNodeBehaviourEvent::Messaging(messaging_event) => self.extended_metrics.record(messaging_event),
            TariNodeBehaviourEvent::Substream(substream_event) => self.extended_metrics.record(substream_event),
            TariNodeBehaviourEvent::ConnectionLimits(_) => {}, // No events for connection limits
            TariNodeBehaviourEvent::InboundLimits(_) => {},    // No events for inbound limits
            TariNodeBehaviourEvent::Mdns(_) => {},             // No metrics for mDNS events
            TariNodeBehaviourEvent::Autonat(_) => {},          // No metrics for Autonat events
        }
    }
}

/// Every swarm event must be recorded: libp2p's identify metrics retain each identified peer until the event closing
/// its last connection is recorded.
impl<TCodec: libp2p_messaging::Codec + Send + Clone + 'static> Recorder<SwarmEvent<TariNodeBehaviourEvent<TCodec>>>
    for Metrics
{
    fn record(&self, event: &SwarmEvent<TariNodeBehaviourEvent<TCodec>>) {
        self.libp2p_metrics.record(event);
        if let SwarmEvent::Behaviour(event) = event {
            self.record(event);
        }
    }
}

pub struct ExtendedMetrics {
    substream: libp2p_substream::metrics::Metrics,
    messaging: libp2p_messaging::metrics::Metrics,
    rendezvous: RendezvousMetrics,
}

impl ExtendedMetrics {
    pub fn new(registry: &mut libp2p::metrics::Registry) -> Self {
        let registry = registry.sub_registry_with_prefix("libp2p_ext");
        Self {
            substream: libp2p_substream::metrics::Metrics::new(registry),
            messaging: libp2p_messaging::metrics::Metrics::new(registry),
            rendezvous: RendezvousMetrics::new(registry),
        }
    }
}

impl Recorder<libp2p_substream::Event> for ExtendedMetrics {
    fn record(&self, event: &libp2p_substream::Event) {
        self.substream.record(event);
    }
}

impl<TMsg> Recorder<libp2p_messaging::Event<TMsg>> for ExtendedMetrics {
    fn record(&self, event: &libp2p_messaging::Event<TMsg>) {
        self.messaging.record(event);
    }
}

impl Recorder<rendezvous::client::Event> for ExtendedMetrics {
    fn record(&self, event: &rendezvous::client::Event) {
        self.rendezvous.record_client(event);
    }
}

impl Recorder<rendezvous::server::Event> for ExtendedMetrics {
    fn record(&self, event: &rendezvous::server::Event) {
        self.rendezvous.record_server(event);
    }
}

/// Metrics for the rendezvous client and server behaviours. libp2p does not ship metrics for rendezvous, so these are
/// maintained here.
struct RendezvousMetrics {
    client_registered: Counter,
    client_register_failed: Counter,
    client_discovered: Counter,
    client_discovered_peers: Counter,
    client_discover_failed: Counter,
    client_expired: Counter,
    server_peer_registered: Counter,
    server_peer_not_registered: Counter,
    server_peer_unregistered: Counter,
    server_registration_expired: Counter,
    server_discover_served: Counter,
    server_discover_not_served: Counter,
}

impl RendezvousMetrics {
    fn new(registry: &mut libp2p::metrics::Registry) -> Self {
        let registry = registry.sub_registry_with_prefix("rendezvous");

        let client_registered = Counter::default();
        registry.register(
            "client_registered",
            "Number of successful registrations with a rendezvous server",
            client_registered.clone(),
        );
        let client_register_failed = Counter::default();
        registry.register(
            "client_register_failed",
            "Number of failed registrations with a rendezvous server",
            client_register_failed.clone(),
        );
        let client_discovered = Counter::default();
        registry.register(
            "client_discovered",
            "Number of discover responses received from rendezvous servers",
            client_discovered.clone(),
        );
        let client_discovered_peers = Counter::default();
        registry.register(
            "client_discovered_peers",
            "Total number of peer registrations discovered via rendezvous",
            client_discovered_peers.clone(),
        );
        let client_discover_failed = Counter::default();
        registry.register(
            "client_discover_failed",
            "Number of failed discover requests to rendezvous servers",
            client_discover_failed.clone(),
        );
        let client_expired = Counter::default();
        registry.register(
            "client_expired",
            "Number of times a discovered peer's details expired",
            client_expired.clone(),
        );

        let server_peer_registered = Counter::default();
        registry.register(
            "server_peer_registered",
            "Number of peers that successfully registered with us",
            server_peer_registered.clone(),
        );
        let server_peer_not_registered = Counter::default();
        registry.register(
            "server_peer_not_registered",
            "Number of registrations we declined",
            server_peer_not_registered.clone(),
        );
        let server_peer_unregistered = Counter::default();
        registry.register(
            "server_peer_unregistered",
            "Number of peers that unregistered with us",
            server_peer_unregistered.clone(),
        );
        let server_registration_expired = Counter::default();
        registry.register(
            "server_registration_expired",
            "Number of registrations that expired",
            server_registration_expired.clone(),
        );
        let server_discover_served = Counter::default();
        registry.register(
            "server_discover_served",
            "Number of discover requests we served",
            server_discover_served.clone(),
        );
        let server_discover_not_served = Counter::default();
        registry.register(
            "server_discover_not_served",
            "Number of discover requests we failed to serve",
            server_discover_not_served.clone(),
        );

        Self {
            client_registered,
            client_register_failed,
            client_discovered,
            client_discovered_peers,
            client_discover_failed,
            client_expired,
            server_peer_registered,
            server_peer_not_registered,
            server_peer_unregistered,
            server_registration_expired,
            server_discover_served,
            server_discover_not_served,
        }
    }

    fn record_client(&self, event: &rendezvous::client::Event) {
        match event {
            rendezvous::client::Event::Registered { .. } => {
                self.client_registered.inc();
            },
            rendezvous::client::Event::RegisterFailed { .. } => {
                self.client_register_failed.inc();
            },
            rendezvous::client::Event::Discovered { registrations, .. } => {
                self.client_discovered.inc();
                self.client_discovered_peers.inc_by(registrations.len() as u64);
            },
            rendezvous::client::Event::DiscoverFailed { .. } => {
                self.client_discover_failed.inc();
            },
            rendezvous::client::Event::Expired { .. } => {
                self.client_expired.inc();
            },
        }
    }

    fn record_server(&self, event: &rendezvous::server::Event) {
        match event {
            rendezvous::server::Event::PeerRegistered { .. } => {
                self.server_peer_registered.inc();
            },
            rendezvous::server::Event::PeerNotRegistered { .. } => {
                self.server_peer_not_registered.inc();
            },
            rendezvous::server::Event::PeerUnregistered { .. } => {
                self.server_peer_unregistered.inc();
            },
            rendezvous::server::Event::RegistrationExpired(_) => {
                self.server_registration_expired.inc();
            },
            rendezvous::server::Event::DiscoverServed { .. } => {
                self.server_discover_served.inc();
            },
            rendezvous::server::Event::DiscoverNotServed { .. } => {
                self.server_discover_not_served.inc();
            },
        }
    }
}
