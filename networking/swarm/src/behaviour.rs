//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use libp2p::{
    autonat,
    connection_limits,
    connection_limits::ConnectionLimits,
    dcutr,
    gossipsub,
    identify,
    identity::Keypair,
    kad,
    kad::store::MemoryStore,
    mdns,
    noise,
    ping,
    relay,
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour},
    tcp,
    yamux,
    PeerId,
    StreamProtocol,
    Swarm,
    SwarmBuilder,
};
use libp2p_messaging as messaging;
use libp2p_substream as substream;

use crate::{config::Config, error::TariSwarmError};

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
    pub kad: kad::Behaviour<MemoryStore>,
    pub mdns: Toggle<mdns::tokio::Behaviour>,

    pub substream: substream::Behaviour,
    pub messaging: messaging::Behaviour<TCodec>,
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
        .with_tcp(
            tcp::Config::new().nodelay(true).port_reuse(true),
            noise_config,
            yamux::Config::default,
        )?
        .with_quic()
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(|keypair, relay_client| {
            let local_peer_id = keypair.public().to_peer_id();

            // Gossipsub
            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .validation_mode(gossipsub::ValidationMode::Strict) // This sets the kind of message validation. The default is Strict (enforce message signing)
                .message_id_fn(message_id_concat_peer_and_seq_no) // content-address messages. No two messages of the same content will be propagated.
                .build()
                .unwrap();

            let gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(keypair.clone()),
                gossipsub_config,
            )
            .unwrap();

            // Ping
            let ping = ping::Behaviour::new(config.ping);

            // Dcutr
            let dcutr = dcutr::Behaviour::new(local_peer_id);

            // Identify
            let identify = identify::Behaviour::new(
                identify::Config::new(config.protocol_version.to_string(), keypair.public())
                    .with_agent_version(config.user_agent),
            );

            // Kad
            let kad = kad::Behaviour::new(local_peer_id, MemoryStore::new(local_peer_id));

            // Relay
            let maybe_relay = if config.enable_relay {
                Some(relay::Behaviour::new(local_peer_id, relay::Config::default()))
            } else {
                None
            };

            // Messaging
            let messaging = messaging::Behaviour::new(
                StreamProtocol::try_from_owned(config.messaging_protocol)?,
                messaging::Config::default(),
            );

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

            Ok(TariNodeBehaviour {
                ping,
                dcutr,
                identify,
                relay: Toggle::from(maybe_relay),
                relay_client,
                autonat,
                kad,
                gossipsub,
                substream,
                messaging,
                connection_limits,
                mdns: Toggle::from(maybe_mdns),
            })
        })
        .map_err(|e| TariSwarmError::BehaviourError(e.to_string()))?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(config.idle_connection_timeout))
        .build();

    Ok(swarm)
}

fn message_id_concat_peer_and_seq_no(message: &gossipsub::Message) -> gossipsub::MessageId {
    let mut msg_id = if let Some(peer_id) = message.source.as_ref() {
        peer_id.to_bytes()
    } else {
        PeerId::from_bytes(&[0, 1, 0]).expect("Valid peer id").to_bytes()
    };
    msg_id.extend(message.sequence_number.unwrap_or_default().to_be_bytes());
    gossipsub::MessageId::from(msg_id)
}

fn noise_config(keypair: &Keypair) -> Result<noise::Config, noise::Error> {
    Ok(noise::Config::new(keypair)?.with_prologue(noise_prologue()))
}

fn noise_prologue() -> Vec<u8> {
    const PROLOGUE: &str = "tari-digital-asset-network";
    PROLOGUE.as_bytes().to_vec()
}
