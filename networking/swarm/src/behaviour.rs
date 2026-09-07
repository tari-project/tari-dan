//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashSet, hash_map::DefaultHasher},
    hash::Hasher,
};

use libp2p::{
    StreamProtocol,
    Swarm,
    SwarmBuilder,
    autonat,
    connection_limits,
    connection_limits::ConnectionLimits,
    dcutr,
    gossipsub,
    identify,
    identity::Keypair,
    mdns,
    noise,
    ping,
    relay,
    rendezvous,
    swarm::{NetworkBehaviour, behaviour::toggle::Toggle},
    tcp,
    yamux,
};
use libp2p_messaging as messaging;
use libp2p_peer_store as peer_store;
use libp2p_peer_store::memory_store::MemoryStore;
use libp2p_substream as substream;

use crate::{
    config::{Config, RelayCircuitLimits, RelayReservationLimits},
    error::TariSwarmError,
};

pub type PeerData = ();

#[derive(NetworkBehaviour)]
pub struct TariNodeBehaviour<TCodec>
where TCodec: messaging::Codec + Send + Clone + 'static
{
    pub ping: ping::Behaviour,
    pub dcutr: dcutr::Behaviour,
    pub connection_limits: connection_limits::Behaviour,

    pub relay: Toggle<relay::Behaviour>,
    pub relay_client: relay::client::Behaviour,
    pub autonat: autonat::Behaviour,

    pub identify: identify::Behaviour,
    pub mdns: Toggle<mdns::tokio::Behaviour>,
    pub rendezvous_server: Toggle<rendezvous::server::Behaviour>,
    pub rendezvous_client: rendezvous::client::Behaviour,
    pub peer_store: peer_store::Behaviour<MemoryStore<PeerData>>,

    pub substream: substream::Behaviour,
    pub messaging: Toggle<messaging::Behaviour<TCodec>>,
    pub gossipsub: gossipsub::Behaviour,
}

/// Returns true if the given Multiaddr is supported by the Tari swarm, otherwise false.
/// NOTE: this function only currently returns false for onion addresses.
pub fn is_supported_multiaddr(addr: &libp2p::Multiaddr) -> bool {
    !addr.iter().any(|p| {
        matches!(
            p,
            libp2p::core::multiaddr::Protocol::Onion(_, _) | libp2p::core::multiaddr::Protocol::Onion3(_)
        )
    })
}

pub fn create_swarm<TCodec>(
    identity: Keypair,
    supported_protocols: HashSet<StreamProtocol>,
    config: Config,
) -> Result<Swarm<TariNodeBehaviour<TCodec>>, TariSwarmError>
where
    TCodec: messaging::Codec + Clone + Send + 'static,
{
    let swarm = SwarmBuilder::with_existing_identity(identity)
        .with_tokio()
        .with_tcp(tcp::Config::new().nodelay(true), noise_config, yamux::Config::default)?
        .with_quic()
        .with_relay_client(noise_config, yamux::Config::default)?
        .with_behaviour(|keypair, relay_client| {
            let local_peer_id = keypair.public().to_peer_id();

            // Gossipsub
            // Everything gossipsub retains is set here rather than left to a library default: the
            // message cache, the duplicate cache and the per-connection send queue are all bounds
            // on how much memory a flood of gossip can make the node hold, and a bound nobody chose
            // is not a bound. See `Config` for what each one trades away.
            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .max_transmit_size(config.gossip_sub_max_message_size)
                .validation_mode(gossipsub::ValidationMode::Strict) // This sets the kind of message validation. The default is Strict (enforce message signing)
                .validate_messages()
                .message_id_fn(get_message_id) // content-address messages. No two messages of the same content will be propagated.
                .history_length(config.gossip_sub_history_length)
                .history_gossip(config.gossip_sub_history_gossip)
                .duplicate_cache_time(config.gossip_sub_duplicate_cache_time)
                .connection_handler_queue_len(config.gossip_sub_max_send_queue_messages)
                .build()?;

            let mut gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(keypair.clone()),
                gossipsub_config,
            )
            .unwrap();

            if !config.gossip_sub_scored_topics.is_empty() {
                let (params, thresholds) = peer_score_params(&config.gossip_sub_scored_topics);
                // Only fails if the parameters are invalid or scoring is already active, both of
                // which are constructed here.
                gossipsub.with_peer_score(params, thresholds).unwrap();
            }

            // Ping
            let ping = ping::Behaviour::new(config.ping);

            // Dcutr
            let dcutr = dcutr::Behaviour::new(local_peer_id);

            // Relay
            let maybe_relay = if config.enable_relay {
                Some(relay::Behaviour::new(
                    local_peer_id,
                    create_relay_config(&config.relay_circuit_limits, &config.relay_reservation_limits),
                ))
            } else {
                None
            };

            // Identify
            let identify = identify::Behaviour::new(
                identify::Config::new(config.protocol_version.to_string(), keypair.public())
                    .with_interval(config.identify_interval)
                    .with_agent_version(config.user_agent),
            );

            // Messaging
            let messaging = if config.enable_messaging {
                Some(messaging::Behaviour::new(
                    StreamProtocol::try_from_owned(config.messaging_protocol)?,
                    messaging::Config::default(),
                ))
            } else {
                None
            };

            // Substreams
            let substream = substream::Behaviour::new(supported_protocols, substream::Config::default());

            // Connection limits
            let connection_limits = connection_limits::Behaviour::new(
                ConnectionLimits::default().with_max_established_per_peer(config.max_connections_per_peer),
            );

            // mDNS
            let maybe_mdns = if config.enable_mdns {
                Some(mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?)
            } else {
                None
            };

            // autonat
            let autonat = autonat::Behaviour::new(local_peer_id, autonat::Config::default());

            // Rendezvous server
            let rendezvous_server = if config.rendezvous_server_enabled {
                Some(rendezvous::server::Behaviour::new(rendezvous::server::Config::default()))
            } else {
                None
            };

            // Rendezvous client
            let rendezvous_client = rendezvous::client::Behaviour::new(keypair.clone());

            // Peer store
            let peer_store = peer_store::Behaviour::new(MemoryStore::new(peer_store::memory_store::Config::default()));

            Ok(TariNodeBehaviour {
                ping,
                dcutr,
                identify,
                relay: Toggle::from(maybe_relay),
                relay_client,
                autonat,
                gossipsub,
                substream,
                messaging: Toggle::from(messaging),
                connection_limits,
                mdns: Toggle::from(maybe_mdns),
                peer_store,
                rendezvous_server: Toggle::from(rendezvous_server),
                rendezvous_client,
            })
        })
        .map_err(|e| TariSwarmError::BehaviourError(e.to_string()))?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(config.idle_connection_timeout))
        .build();

    Ok(swarm)
}

fn create_relay_config(circuit: &RelayCircuitLimits, reservations: &RelayReservationLimits) -> relay::Config {
    let mut config = relay::Config {
        reservation_rate_limiters: vec![],
        circuit_src_rate_limiters: vec![],
        ..Default::default()
    };

    config.max_circuits = circuit.max_limit;
    config.max_circuits_per_peer = circuit.max_per_peer;
    config.max_circuit_duration = circuit.max_duration;
    config.max_circuit_bytes = circuit.max_byte_limit;
    if let Some(ref limits) = circuit.per_peer {
        config = config.circuit_src_per_peer(limits.limit, limits.interval);
    }

    if let Some(ref limits) = circuit.per_ip {
        config = config.circuit_src_per_ip(limits.limit, limits.interval);
    }

    config.max_reservations = reservations.max_limit;
    config.max_reservations_per_peer = reservations.max_per_peer;
    config.reservation_duration = reservations.max_duration;
    if let Some(ref limits) = reservations.per_peer {
        config = config.reservation_rate_per_peer(limits.limit, limits.interval);
    }

    if let Some(ref limits) = reservations.per_ip {
        config = config.reservation_rate_per_ip(limits.limit, limits.interval);
    }

    config
}

/// Generates a hash of contents of the message
fn get_message_id(message: &gossipsub::Message) -> gossipsub::MessageId {
    let mut hasher = DefaultHasher::new();
    hasher.write(&message.data);
    hasher.write(message.topic.as_str().as_bytes());
    gossipsub::MessageId::from(hasher.finish().to_be_bytes())
}

fn noise_config(keypair: &Keypair) -> Result<noise::Config, noise::Error> {
    Ok(noise::Config::new(keypair)?.with_prologue(noise_prologue()))
}

fn noise_prologue() -> Vec<u8> {
    const PROLOGUE: &str = "tari-digital-asset-network";
    PROLOGUE.as_bytes().to_vec()
}

/// Peer-scoring parameters for the topics in [`Config::gossip_sub_scored_topics`].
///
/// Deliberately narrow: of gossipsub's seven scoring parameters only P4 (invalid message
/// deliveries) and P7 (protocol-level misbehaviour) are enabled.
///
/// P1–P3b score *delivery rates* — how long a peer has been in the mesh, how often it delivers a
/// message first, whether it meets a delivery quota. Their defaults assume a busy topic:
/// `mesh_message_deliveries_threshold` of 20 penalises any peer delivering fewer than 20 messages
/// per window, which on a quiet network is every honest peer. Scoring liveness that way would
/// punish peers for the network being idle, so they are disabled rather than tuned.
///
/// P6 (IP colocation) is omitted. It exists to make Sybil identities cost IP addresses, which
/// matters where identity is free; here consensus participation is gated by validator registration
/// on the base layer, so how an operator distributes their nodes says nothing about whether they
/// are one entity or many. Penalising colocation would tax honest deployments — a local swarm on
/// one host, nodes sharing a NAT egress address — for no signal. Its residual value is bounding
/// eclipse of the gossip mesh, which is accepted: that degrades propagation but not consensus
/// safety, since intra-committee traffic uses direct messaging to peers drawn from the registered
/// set rather than from mesh selection.
///
/// That leaves P4 carrying the signal the application actually produces: a `Reject` verdict from
/// `report_gossip_validation`, reported for messages that fail to decode or fail validation. The
/// penalty is the square of a decaying counter, so occasional invalid messages (version skew, an
/// epoch-boundary race) are forgiven within seconds, while a sustained flood accumulates. At these
/// weights a peer must sustain roughly twenty invalid messages per second to be graylisted.
///
/// The thresholds are libp2p's defaults. Scoring too aggressively partitions a network by
/// graylisting honest peers, which is a far worse failure than scoring too leniently — so this
/// starts permissive, and should be tightened against observed behaviour rather than guessed at.
fn peer_score_params(topics: &[String]) -> (gossipsub::PeerScoreParams, gossipsub::PeerScoreThresholds) {
    let topic_params = gossipsub::TopicScoreParams {
        topic_weight: 1.0,
        time_in_mesh_weight: 0.0,
        first_message_deliveries_weight: 0.0,
        mesh_message_deliveries_weight: 0.0,
        mesh_failure_penalty_weight: 0.0,
        invalid_message_deliveries_weight: -1.0,
        ..Default::default()
    };

    let params = gossipsub::PeerScoreParams {
        topics: topics
            .iter()
            .map(|topic| (gossipsub::IdentTopic::new(topic).hash(), topic_params.clone()))
            .collect(),
        ip_colocation_factor_weight: 0.0,
        ..Default::default()
    };

    (params, gossipsub::PeerScoreThresholds::default())
}
