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

use std::{fs, io, sync::Arc};

use tari_app_utilities::{identity_management, identity_management::load_from_json};
use tari_common::exit_codes::{ExitCode, ExitError};
use tari_comms::{CommsNode, NodeIdentity};
use tari_dan_core::storage::global::GlobalDb;
use tari_dan_storage_sqlite::{global::SqliteGlobalDbBackendAdapter, SqliteDbFactory};
use tari_p2p::initialization::spawn_comms_using_transport;
use tari_shutdown::ShutdownSignal;

use crate::{
    base_layer_scanner::BaseLayerScanner,
    comms,
    grpc::services::base_node_client::GrpcBaseNodeClient,
    p2p::services::{epoch_manager, hotstuff, mempool, messaging, messaging::DanMessageReceivers, template_manager},
    ApplicationConfig,
};

pub async fn spawn_services(
    config: &ApplicationConfig,
    shutdown: ShutdownSignal,
    node_identity: Arc<NodeIdentity>,
    global_db: GlobalDb<SqliteGlobalDbBackendAdapter>,
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
    let outbound_messaging = messaging::spawn(node_identity.public_key().clone(), message_channel, message_senders);

    let DanMessageReceivers {
        rx_consensus_message,
        rx_vote_message,
        rx_new_transaction_message,
    } = message_receivers;

    // Epoch manager
    let epoch_manager_handle =
        epoch_manager::spawn(base_node_client, node_identity.public_key().clone(), shutdown.clone());

    // Mempool
    let mempool = mempool::spawn(rx_new_transaction_message, outbound_messaging.clone());

    // Template manager
    let template_manager = template_manager::spawn(sqlite_db, shutdown.clone());

    // Base Node scanner
    let base_node_client = GrpcBaseNodeClient::new(config.validator_node.base_node_grpc_address);

    let base_layer_scanner = BaseLayerScanner::new(
        config.validator_node.clone(),
        global_db.clone(),
        base_node_client,
        epoch_manager_handle.clone(),
        template_manager,
        shutdown.clone(),
    );

    base_layer_scanner
        .start()
        .await
        .map_err(|err| ExitError::new(ExitCode::DigitalAssetError, err))?;

    // Consensus
    hotstuff::spawn(
        node_identity,
        outbound_messaging,
        epoch_manager_handle,
        mempool,
        rx_consensus_message,
        rx_vote_message,
        shutdown,
    );

    // let comms = setup_p2p_rpc(config, comms);
    let comms = spawn_comms_using_transport(comms, p2p_config.transport.clone())
        .await
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Could not spawn using transport: {}", e)))?;

    // Save final node identity after comms has initialized. This is required because the public_address can be
    // changed by comms during initialization when using tor.
    identity_management::save_as_json(&config.validator_node.identity_file, &*comms.node_identity())
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Failed to save node identity: {}", e)))?;
    if let Some(hs) = comms.hidden_service() {
        identity_management::save_as_json(&config.validator_node.tor_identity_file, hs.tor_identity())
            .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Failed to save tor identity: {}", e)))?;
    }

    Ok(Services { comms })
}

fn ensure_directories_exist(config: &ApplicationConfig) -> io::Result<()> {
    fs::create_dir_all(&config.validator_node.data_dir)?;
    fs::create_dir_all(&config.validator_node.p2p.datastore_path)?;
    Ok(())
}

pub struct Services {
    pub comms: CommsNode,
    // TODO: Add more as needed
}

// fn setup_p2p_rpc(config: &ApplicationConfig, comms: UnspawnedCommsNode) -> UnspawnedCommsNode {
//     let rpc_server = RpcServer::builder()
//         .with_maximum_simultaneous_sessions(config.validator_node.p2p.rpc_max_simultaneous_sessions)
//         // .add_service(create_validator_node_rpc_service(mempool, db_factory));
//         .finish();
//
//     comms.add_protocol_extension(rpc_server)
// }
