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

use std::{
    fs,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

use tari_app_utilities::{identity_management, identity_management::load_from_json};
use tari_common::{
    configuration::bootstrap::{grpc_default_port, ApplicationType},
    exit_codes::{ExitCode, ExitError},
};
use tari_comms::{protocol::rpc::RpcServer, CommsNode, NodeIdentity, UnspawnedCommsNode};
use tari_dan_core::{
    consensus_constants::ConsensusConstants,
    workers::events::{EventSubscription, HotStuffEvent},
};
use tari_dan_storage::global::GlobalDb;
use tari_dan_storage_sqlite::{global::SqliteGlobalDbAdapter, sqlite_shard_store_factory::SqliteShardStore};
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
            messaging::DanMessageReceivers,
            networking,
            networking::NetworkingHandle,
            rpc_client::TariCommsValidatorNodeClientFactory,
            template_manager,
            template_manager::{TemplateManager, TemplateManagerHandle},
        },
    },
    payload_processor::TariDanPayloadProcessor,
    registration,
    ApplicationConfig,
};

const _LOG_TARGET: &str = "tari_validator_node::bootstrap";

pub async fn spawn_services(
    config: &ApplicationConfig,
    shutdown: ShutdownSignal,
    node_identity: Arc<NodeIdentity>,
    global_db: GlobalDb<SqliteGlobalDbAdapter>,
    consensus_constants: ConsensusConstants,
) -> Result<Services, anyhow::Error> {
    let mut p2p_config = config.validator_node.p2p.clone();
    p2p_config.transport.tor.identity = load_from_json(&config.validator_node.tor_identity_file)
        .map_err(|e| ExitError::new(ExitCode::ConfigError, e))?;

    ensure_directories_exist(config)?;

    // Connection to base node
    let base_node_client = GrpcBaseNodeClient::new(config.validator_node.base_node_grpc_address.unwrap_or_else(|| {
        let port = grpc_default_port(ApplicationType::BaseNode, config.network);
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }));

    // Initialize comms
    let (comms, message_channel) = comms::initialize(node_identity.clone(), config, shutdown.clone()).await?;

    // Spawn messaging
    let (message_senders, message_receivers) = messaging::new_messaging_channel(10);
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

    // Connect to shard db
    let shard_store = SqliteShardStore::try_create(config.validator_node.state_db_path())?;

    // Epoch manager
    let validator_node_client_factory = TariCommsValidatorNodeClientFactory::new(comms.connectivity());
    let epoch_manager = epoch_manager::spawn(
        global_db.clone(),
        shard_store.clone(),
        base_node_client.clone(),
        consensus_constants.clone(),
        shutdown.clone(),
        node_identity.clone(),
        validator_node_client_factory,
    );

    // Mempool
    let mempool = mempool::spawn(
        rx_new_transaction_message,
        outbound_messaging.clone(),
        epoch_manager.clone(),
        node_identity.clone(),
    );

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

    let template_manager = TemplateManager::new(global_db.clone(), config.validator_node.templates.clone());
    let template_manager_service = template_manager::spawn(template_manager.clone(), shutdown.clone());

    // Base Node scanner
    base_layer_scanner::spawn(
        config.validator_node.clone(),
        global_db,
        base_node_client.clone(),
        epoch_manager.clone(),
        template_manager_service.clone(),
        shutdown.clone(),
        consensus_constants,
    );

    // Payload processor
    let payload_processor = TariDanPayloadProcessor::new(template_manager);

    // Consensus
    let hotstuff_events = hotstuff::try_spawn(
        node_identity.clone(),
        shard_store.clone(),
        outbound_messaging,
        epoch_manager.clone(),
        mempool.clone(),
        payload_processor,
        rx_consensus_message,
        rx_vote_message,
        shutdown.clone(),
    );

    let comms = setup_p2p_rpc(config, comms, peer_provider, shard_store.clone(), mempool.clone());
    let comms = comms::spawn_comms_using_transport(comms, p2p_config.transport.clone())
        .await
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Could not spawn using transport: {}", e)))?;

    // Save final node identity after comms has initialized. This is required because the public_address can be
    // changed by comms during initialization when using tor.
    save_identities(config, &comms)?;

    // Auto-registration
    registration::spawn(config.clone(), node_identity.clone(), epoch_manager.clone(), shutdown);

    Ok(Services {
        comms,
        networking,
        mempool,
        epoch_manager,
        template_manager: template_manager_service,
        hotstuff_events,
        shard_store,
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
    pub template_manager: TemplateManagerHandle,
    pub hotstuff_events: EventSubscription<HotStuffEvent>,
    pub shard_store: SqliteShardStore,
}

fn setup_p2p_rpc(
    config: &ApplicationConfig,
    comms: UnspawnedCommsNode,
    peer_provider: CommsPeerProvider,
    shard_store_store: SqliteShardStore,
    mempool: MempoolHandle,
) -> UnspawnedCommsNode {
    let rpc_server = RpcServer::builder()
        .with_maximum_simultaneous_sessions(config.validator_node.p2p.rpc_max_simultaneous_sessions)
        .finish()
        .add_service(create_validator_node_rpc_service(
            peer_provider,
            shard_store_store,
            mempool,
        ));

    comms.add_protocol_extension(rpc_server)
}
