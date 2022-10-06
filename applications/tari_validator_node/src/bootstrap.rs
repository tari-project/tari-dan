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

use std::{fs, io, str::FromStr, sync::Arc};

use tari_app_utilities::{identity_management, identity_management::load_from_json};
use tari_common::exit_codes::{ExitCode, ExitError};
use tari_comms::{
    peer_manager::{Peer, PeerFlags},
    protocol::rpc::RpcServer,
    CommsNode,
    NodeIdentity,
    PeerManager,
    UnspawnedCommsNode,
};
use tari_dan_storage::global::GlobalDb;
use tari_dan_storage_sqlite::{global::SqliteGlobalDbAdapter, SqliteDbFactory};
use tari_p2p::{initialization::spawn_comms_using_transport, peer_seeds::SeedPeer, PeerSeedsConfig};
use tari_shutdown::ShutdownSignal;

use crate::{
    base_layer_scanner,
    comms,
    grpc::services::base_node_client::GrpcBaseNodeClient,
    p2p::{
        create_validator_node_rpc_service,
        services::{
            comms_peer_provider::CommsPeerProvider,
            epoch_manager,
            epoch_manager::handle::EpochManagerHandle,
            hotstuff,
            mempool,
            mempool::MempoolHandle,
            messaging,
            messaging::{DanMessageReceivers, DanMessageSenders},
            networking,
            networking::NetworkingHandle,
            template_manager,
            template_manager::manager::TemplateManager,
        },
    },
    payload_processor::TariDanPayloadProcessor,
    ApplicationConfig,
};

const _LOG_TARGET: &str = "tari_validator_node::bootstrap";

pub async fn spawn_services(
    config: &ApplicationConfig,
    shutdown: ShutdownSignal,
    node_identity: Arc<NodeIdentity>,
    global_db: GlobalDb<SqliteGlobalDbAdapter>,
    sqlite_db: SqliteDbFactory,
) -> Result<Services, anyhow::Error> {
    let mut p2p_config = config.validator_node.p2p.clone();
    p2p_config.transport.tor.identity = load_from_json(&config.validator_node.tor_identity_file)
        .map_err(|e| ExitError::new(ExitCode::ConfigError, e))?;

    ensure_directories_exist(config)?;

    // Connection to base node
    let base_node_client = GrpcBaseNodeClient::new(config.validator_node.base_node_grpc_address);

    // Initialize comms
    let (comms, message_channel) = comms::initialize(node_identity.clone(), p2p_config.clone(), shutdown.clone())?;

    // Spawn messaging
    let (message_senders, message_receivers) = messaging::new_messaging_channel(10);
    let mempool_new_tx = message_senders.tx_new_transaction_message.clone();
    let outbound_messaging = messaging::spawn(
        node_identity.public_key().clone(),
        message_channel,
        message_senders.clone(),
    );

    let DanMessageReceivers {
        rx_consensus_message,
        rx_vote_message,
        rx_new_transaction_message,
        rx_network_announce,
    } = message_receivers;

    // Epoch manager
    let epoch_manager = epoch_manager::spawn(
        base_node_client.clone(),
        node_identity.public_key().clone(),
        shutdown.clone(),
    );

    // Mempool
    let mempool = mempool::spawn(rx_new_transaction_message, mempool_new_tx, outbound_messaging.clone());

    // Add seeds
    add_seed_peers(&comms.peer_manager(), &comms.node_identity(), &config.peer_seeds).await?;

    // Networking
    let peer_provider = CommsPeerProvider::new(comms.peer_manager());
    let networking = networking::spawn(
        rx_network_announce,
        node_identity.clone(),
        outbound_messaging.clone(),
        peer_provider.clone(),
        comms.connectivity(),
    );

    // Template manager
    let template_manager = template_manager::spawn(sqlite_db.clone(), shutdown.clone());

    // Base Node scanner
    base_layer_scanner::spawn(
        config.validator_node.clone(),
        global_db.clone(),
        base_node_client.clone(),
        epoch_manager.clone(),
        template_manager.clone(),
        shutdown.clone(),
    );

    // Payload processor
    // TODO: we recreate the db template manager here, we could use the TemplateManagerHandle, but this is async, which
    //       would force the PayloadProcessor to be async (maybe that is ok, or maybe we dont need the template manager
    //       to be async if it doesn't have to download the templates).
    let payload_processor = TariDanPayloadProcessor::new(TemplateManager::new(sqlite_db));

    // Consensus
    hotstuff::spawn(
        node_identity.clone(),
        outbound_messaging,
        epoch_manager.clone(),
        mempool.clone(),
        payload_processor,
        rx_consensus_message,
        rx_vote_message,
        shutdown,
    );

    let comms = setup_p2p_rpc(config, comms, message_senders, peer_provider);
    let comms = spawn_comms_using_transport(comms, p2p_config.transport.clone())
        .await
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Could not spawn using transport: {}", e)))?;

    // Save final node identity after comms has initialized. This is required because the public_address can be
    // changed by comms during initialization when using tor.
    save_identities(config, &comms)?;

    Ok(Services {
        comms,
        networking,
        mempool,
        epoch_manager,
    })
}

fn save_identities(config: &ApplicationConfig, comms: &CommsNode) -> Result<(), ExitError> {
    identity_management::save_as_json(&config.validator_node.identity_file, &*comms.node_identity())
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Failed to save node identity: {}", e)))?;

    if let Some(hs) = comms.hidden_service() {
        identity_management::save_as_json(&config.validator_node.tor_identity_file, hs.tor_identity())
            .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Failed to save tor identity: {}", e)))?;
    }
    Ok(())
}

fn ensure_directories_exist(config: &ApplicationConfig) -> io::Result<()> {
    fs::create_dir_all(&config.validator_node.data_dir)?;
    fs::create_dir_all(&config.validator_node.p2p.datastore_path)?;
    Ok(())
}

pub struct Services {
    pub comms: CommsNode,
    pub networking: NetworkingHandle,
    pub mempool: MempoolHandle,
    pub epoch_manager: EpochManagerHandle,
}

fn setup_p2p_rpc(
    config: &ApplicationConfig,
    comms: UnspawnedCommsNode,
    message_senders: DanMessageSenders,
    peer_provider: CommsPeerProvider,
) -> UnspawnedCommsNode {
    let rpc_server = RpcServer::builder()
        .with_maximum_simultaneous_sessions(config.validator_node.p2p.rpc_max_simultaneous_sessions)
        .finish()
        .add_service(create_validator_node_rpc_service(message_senders, peer_provider));

    comms.add_protocol_extension(rpc_server)
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

        // debug!(target: LOG_TARGET, "Adding seed peer [{}]", peer);
        peer_manager.add_peer(peer).await?;
    }
    Ok(())
}
