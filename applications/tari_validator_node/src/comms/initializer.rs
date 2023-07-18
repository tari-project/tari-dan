//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{str::FromStr, sync::Arc, time::Duration};

use anyhow::anyhow;
use lmdb_zero::open;
use log::*;
use tari_common::configuration::Network;
use tari_comms::{
    backoff::ConstantBackoff,
    peer_manager::{Peer, PeerFeatures, PeerFlags},
    pipeline,
    pipeline::SinkService,
    protocol::{messaging::MessagingProtocolExtension, NodeNetworkInfo},
    tor,
    transports::{predicate::FalsePredicate, MemoryTransport, SocksConfig, SocksTransport, TcpWithTorTransport},
    types::CommsPublicKey,
    utils::cidr::parse_cidrs,
    CommsBuilder,
    CommsNode,
    NodeIdentity,
    PeerManager,
    UnspawnedCommsNode,
};
use tari_comms_logging::SqliteMessageLog;
use tari_dan_p2p::DanMessage;
use tari_p2p::{
    initialization::CommsInitializationError,
    peer_seeds::SeedPeer,
    P2pConfig,
    PeerSeedsConfig,
    TorTransportConfig,
    TransportConfig,
    TransportType,
    MAJOR_NETWORK_VERSION,
    MINOR_NETWORK_VERSION,
};
use tari_shutdown::ShutdownSignal;
use tari_storage::{
    lmdb_store::{LMDBBuilder, LMDBConfig},
    LMDBWrapper,
};
use tokio::sync::{broadcast, mpsc};
use tower::ServiceBuilder;

const LOG_TARGET: &str = "tari::dan::comms::initializer";

use crate::{
    comms::{broadcast::DanBroadcast, deserialize::DanDeserialize, destination::Destination},
    ApplicationConfig,
};

pub async fn initialize(
    node_identity: Arc<NodeIdentity>,
    config: &ApplicationConfig,
    shutdown_signal: ShutdownSignal,
) -> Result<(UnspawnedCommsNode, MessageChannel), anyhow::Error> {
    debug!(target: LOG_TARGET, "Initializing DAN comms");
    let seed_peers = &config.peer_seeds;
    let mut config = config.validator_node.p2p.clone();

    let mut comms_builder = CommsBuilder::new()
        .with_shutdown_signal(shutdown_signal)
        .with_node_identity(node_identity)
        .with_node_info(NodeNetworkInfo {
            major_version: MAJOR_NETWORK_VERSION,
            minor_version: MINOR_NETWORK_VERSION,
            network_byte: Network::Esmeralda.as_byte(), // TODO: DAN has its own network byte?
            user_agent: config.user_agent.clone(),
        });

    if config.allow_test_addresses || config.dht.allow_test_addresses {
        // The default is false, so ensure that both settings are true in this case
        config.allow_test_addresses = true;
        config.dht.allow_test_addresses = true;
        comms_builder = comms_builder.allow_test_addresses();
    }

    let (comms, message_channel) = configure_comms(&config, comms_builder)?;

    // let node_identity = comms.node_identity();

    // TODO: DNS seeds
    // let peers = match Self::try_resolve_dns_seeds(&self.seed_config).await {
    //     Ok(peers) => peers,
    //     Err(err) => {
    //         warn!(target: LOG_TARGET, "Failed to resolve DNS seeds: {}", err);
    //         Vec::new()
    //     },
    // };
    // add_seed_peers(&peer_manager, &node_identity, peers).await?;
    //
    let peer_manager = comms.peer_manager();
    let node_identity = comms.node_identity();
    add_seed_peers(&peer_manager, &node_identity, seed_peers).await?;

    debug!(target: LOG_TARGET, "DAN comms Initialized");
    Ok((comms, message_channel))
}

pub type MessageChannel = (
    mpsc::Sender<(Destination<CommsPublicKey>, DanMessage<CommsPublicKey>)>,
    mpsc::Receiver<(CommsPublicKey, DanMessage<CommsPublicKey>)>,
);

fn configure_comms(
    config: &P2pConfig,
    builder: CommsBuilder,
) -> Result<(UnspawnedCommsNode, MessageChannel), anyhow::Error> {
    let (inbound_tx, inbound_rx) = mpsc::channel(10);
    let (outbound_tx, outbound_rx) = mpsc::channel(10);
    // let file_lock = acquire_exclusive_file_lock(&config.datastore_path)?;

    let datastore = LMDBBuilder::new()
        .set_path(&config.datastore_path)
        .set_env_flags(open::NOLOCK)
        .set_env_config(LMDBConfig::default())
        .set_max_number_of_databases(1)
        .add_database(&config.peer_database_name, lmdb_zero::db::CREATE)
        .build()
        .unwrap();
    let peer_database = datastore.get_handle(&config.peer_database_name).unwrap();
    let peer_database = LMDBWrapper::new(Arc::new(peer_database));

    let listener_liveness_allowlist_cidrs =
        parse_cidrs(&config.listener_liveness_allowlist_cidrs).map_err(|e| anyhow!("{}", e))?;

    let builder = builder
        .set_liveness_check(Some(Duration::from_secs(10)))
        .with_listener_liveness_max_sessions(config.listener_liveness_max_sessions)
        .with_listener_liveness_allowlist_cidrs(listener_liveness_allowlist_cidrs)
        .with_dial_backoff(ConstantBackoff::new(Duration::from_millis(500)))
        .with_peer_storage(peer_database, None);
    // .with_peer_storage(peer_database, Some(file_lock));

    let mut comms = match config.auxiliary_tcp_listener_address {
        Some(ref addr) => builder.with_auxiliary_tcp_listener_address(addr.clone()).build()?,
        None => builder.build()?,
    };

    // Hook up messaging middlewares (currently none)
    let connectivity = comms.connectivity();
    let logger1 = SqliteMessageLog::new(&config.datastore_path);
    let logger2 = logger1.clone();
    let messaging_pipeline = pipeline::Builder::new()
        .with_outbound_pipeline(outbound_rx, move |sink| {
            ServiceBuilder::new()
                .layer(DanBroadcast::new(connectivity, logger1))
                .service(sink)
        })
        .max_concurrent_inbound_tasks(3)
        .max_concurrent_outbound_tasks(3)
        .with_inbound_pipeline(
            ServiceBuilder::new()
                .layer(DanDeserialize::new(comms.peer_manager(), logger2))
                .service(SinkService::new(inbound_tx)),
        )
        .build();

    // TODO: messaging events should be optional
    let (messaging_events_sender, _) = broadcast::channel(1);
    comms = comms.add_protocol_extension(MessagingProtocolExtension::new(
        messaging_events_sender,
        messaging_pipeline,
    ));

    Ok((comms, (outbound_tx, inbound_rx)))
}

async fn add_seed_peers(
    peer_manager: &PeerManager,
    node_identity: &NodeIdentity,
    config: &PeerSeedsConfig,
) -> Result<(), anyhow::Error> {
    let peers = config
        .peer_seeds
        .iter()
        .map(|s| SeedPeer::from_str(s).map(Peer::from))
        .collect::<Result<Vec<_>, _>>()?;

    for mut peer in peers {
        if &peer.public_key == node_identity.public_key() {
            continue;
        }
        peer.add_flags(PeerFlags::SEED);
        peer.set_features(PeerFeatures::COMMUNICATION_NODE);

        // debug!(target: LOG_TARGET, "Adding seed peer [{}]", peer);
        peer_manager.add_peer(peer).await?;
    }
    Ok(())
}

pub async fn spawn_comms_using_transport(
    comms: UnspawnedCommsNode,
    transport_config: TransportConfig,
) -> Result<CommsNode, CommsInitializationError> {
    let comms = match transport_config.transport_type {
        TransportType::Memory => {
            debug!(target: LOG_TARGET, "Building in-memory comms stack");
            comms
                .with_listener_address(transport_config.memory.listener_address.clone())
                .spawn_with_transport(MemoryTransport)
                .await?
        },
        TransportType::Tcp => {
            let config = transport_config.tcp;
            debug!(
                target: LOG_TARGET,
                "Building TCP comms stack{}",
                config
                    .tor_socks_address
                    .as_ref()
                    .map(|_| " with Tor support")
                    .unwrap_or("")
            );
            let mut transport = TcpWithTorTransport::new();
            if let Some(addr) = config.tor_socks_address {
                transport.set_tor_socks_proxy(SocksConfig {
                    proxy_address: addr,
                    authentication: config.tor_socks_auth.into(),
                    proxy_bypass_predicate: Arc::new(FalsePredicate::new()),
                });
            }
            comms
                .with_listener_address(config.listener_address)
                .spawn_with_transport(transport)
                .await?
        },
        TransportType::Tor => {
            let tor_config = transport_config.tor;
            debug!(target: LOG_TARGET, "Building TOR comms stack ({:?})", tor_config);
            let mut hidden_service_ctl = initialize_hidden_service(tor_config).await?;
            // Set the listener address to be the address (usually local) to which tor will forward all traffic
            let transport = hidden_service_ctl.initialize_transport().await?;
            debug!(target: LOG_TARGET, "Comms and DHT configured");
            comms
                .with_listener_address(hidden_service_ctl.proxied_address())
                .with_hidden_service_controller(hidden_service_ctl)
                .spawn_with_transport(transport)
                .await?
        },
        TransportType::Socks5 => {
            debug!(target: LOG_TARGET, "Building SOCKS5 comms stack");
            let transport = SocksTransport::new(transport_config.socks.into());
            comms
                .with_listener_address(transport_config.tcp.listener_address)
                .spawn_with_transport(transport)
                .await?
        },
    };

    Ok(comms)
}

async fn initialize_hidden_service(
    mut config: TorTransportConfig,
) -> Result<tor::HiddenServiceController, CommsInitializationError> {
    let mut builder = tor::HiddenServiceBuilder::new()
        .with_hs_flags(tor::HsFlags::DETACH)
        .with_port_mapping(config.to_port_mapping()?)
        .with_socks_authentication(config.to_socks_auth())
        .with_control_server_auth(config.to_control_auth()?)
        .with_socks_address_override(config.socks_address_override)
        .with_control_server_address(config.control_address)
        .with_bypass_proxy_addresses(config.proxy_bypass_addresses.into());

    if config.proxy_bypass_for_outbound_tcp {
        builder = builder.bypass_tor_for_tcp_addresses();
    }

    if let Some(identity) = config.identity.take() {
        builder = builder.with_tor_identity(identity);
    }

    let hidden_svc_ctl = builder.build()?;
    Ok(hidden_svc_ctl)
}
