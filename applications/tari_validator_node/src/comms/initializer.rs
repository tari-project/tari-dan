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
    types::CommsPublicKey,
    utils::cidr::parse_cidrs,
    CommsBuilder,
    NodeIdentity,
    PeerManager,
    UnspawnedCommsNode,
};
use tari_dan_core::{message::DanMessage, models::TariDanPayload};
use tari_p2p::{peer_seeds::SeedPeer, P2pConfig, PeerSeedsConfig, MAJOR_NETWORK_VERSION, MINOR_NETWORK_VERSION};
use tari_shutdown::ShutdownSignal;
use tari_storage::{
    lmdb_store::{LMDBBuilder, LMDBConfig},
    LMDBWrapper,
};
use tokio::sync::{broadcast, mpsc};
use tower::ServiceBuilder;

const LOG_TARGET: &str = "dan::comms::initializer";

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
    add_seed_peers(&comms.peer_manager(), &comms.node_identity(), seed_peers).await?;

    debug!(target: LOG_TARGET, "DAN comms Initialized");
    Ok((comms, message_channel))
}

pub type MessageChannel = (
    mpsc::Sender<(Destination<CommsPublicKey>, DanMessage<TariDanPayload, CommsPublicKey>)>,
    mpsc::Receiver<(CommsPublicKey, DanMessage<TariDanPayload, CommsPublicKey>)>,
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
    let messaging_pipeline = pipeline::Builder::new()
        .with_outbound_pipeline(outbound_rx, move |sink| {
            ServiceBuilder::new()
                .layer(DanBroadcast::new(connectivity))
                .service(sink)
        })
        .max_concurrent_inbound_tasks(3)
        .max_concurrent_outbound_tasks(3)
        .with_inbound_pipeline(
            ServiceBuilder::new()
                .layer(DanDeserialize::new(comms.peer_manager()))
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
