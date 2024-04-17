//   Copyright 2023. The Tari Project
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

use std::{fs, io, str::FromStr};

use libp2p::identity;
use minotari_app_utilities::identity_management;
use tari_base_node_client::grpc::GrpcBaseNodeClient;
use tari_common::{
    configuration::bootstrap::{grpc_default_port, ApplicationType},
    exit_codes::{ExitCode, ExitError},
};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_app_utilities::{
    base_layer_scanner,
    consensus_constants::ConsensusConstants,
    keypair::RistrettoKeypair,
    seed_peer::SeedPeer,
    template_manager::{self, implementation::TemplateManager},
};
use tari_dan_common_types::PeerAddress;
use tari_dan_p2p::TariMessagingSpec;
use tari_dan_storage::global::GlobalDb;
use tari_dan_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_epoch_manager::base_layer::{EpochManagerConfig, EpochManagerHandle};
use tari_networking::{MessagingMode, NetworkingHandle, SwarmConfig};
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tari_validator_node_rpc::client::TariValidatorNodeRpcClientFactory;

use crate::{substate_storage_sqlite::sqlite_substate_store_factory::SqliteSubstateStore, ApplicationConfig};

const _LOG_TARGET: &str = "tari_indexer::bootstrap";

pub async fn spawn_services(
    config: &ApplicationConfig,
    shutdown: ShutdownSignal,
    keypair: RistrettoKeypair,
    global_db: GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>,
    consensus_constants: ConsensusConstants,
) -> Result<Services, anyhow::Error> {
    ensure_directories_exist(config)?;

    // GRPC client connection to base node
    let base_node_client =
        GrpcBaseNodeClient::new(config.indexer.base_node_grpc_address.clone().unwrap_or_else(|| {
            let port = grpc_default_port(ApplicationType::BaseNode, config.network);
            format!("127.0.0.1:{port}")
        }));

    // Initialize networking
    let identity = identity::Keypair::sr25519_from_bytes(keypair.secret_key().as_bytes().to_vec()).map_err(|e| {
        ExitError::new(
            ExitCode::ConfigError,
            format!("Failed to create libp2p identity from secret bytes: {}", e),
        )
    })?;
    let seed_peers = config
        .peer_seeds
        .peer_seeds
        .iter()
        .map(|s| SeedPeer::from_str(s))
        .collect::<anyhow::Result<Vec<SeedPeer>>>()?;
    let seed_peers = seed_peers
        .into_iter()
        .flat_map(|p| {
            let peer_id = p.to_peer_id();
            p.addresses.into_iter().map(move |a| (peer_id, a))
        })
        .collect();
    let (networking, _) = tari_networking::spawn::<TariMessagingSpec>(
        identity,
        MessagingMode::Disabled,
        tari_networking::Config {
            listener_port: config.indexer.p2p.listener_port,
            swarm: SwarmConfig {
                protocol_version: format!("/tari/{}/0.0.1", config.network).parse().unwrap(),
                user_agent: "/tari/indexer/0.0.1".to_string(),
                enable_mdns: config.indexer.p2p.enable_mdns,
                ..Default::default()
            },
            reachability_mode: config.indexer.p2p.reachability_mode.into(),
            announce: false,
            ..Default::default()
        },
        seed_peers,
        shutdown.clone(),
    )?;

    // Connect to substate db
    let substate_store = SqliteSubstateStore::try_create(config.indexer.state_db_path())?;

    // Epoch manager
    let validator_node_client_factory = TariValidatorNodeRpcClientFactory::new(networking.clone());
    let (epoch_manager, _) = tari_epoch_manager::base_layer::spawn_service(
        EpochManagerConfig {
            base_layer_confirmations: consensus_constants.base_layer_confirmations,
            committee_size: consensus_constants.committee_size,
        },
        global_db.clone(),
        base_node_client.clone(),
        keypair.public_key().clone(),
        shutdown.clone(),
    );

    // Template manager
    let template_manager = TemplateManager::initialize(global_db.clone(), config.indexer.templates.clone())?;
    let (template_manager_service, _) =
        template_manager::implementation::spawn(template_manager.clone(), shutdown.clone());

    // Base Node scanner
    base_layer_scanner::spawn(
        config.network,
        global_db,
        base_node_client.clone(),
        epoch_manager.clone(),
        template_manager_service.clone(),
        shutdown.clone(),
        consensus_constants,
        // TODO: Remove coupling between scanner and shard store
        SqliteStateStore::connect(&format!(
            "sqlite://{}",
            config.indexer.data_dir.join("unused-shard-store.sqlite").display()
        ))?,
        true,
        config.indexer.base_layer_scanning_interval,
    );

    // Save final node identity after comms has initialized. This is required because the public_address can be
    // changed by comms during initialization when using tor.
    save_identities(config, &keypair)?;
    Ok(Services {
        keypair,
        networking,
        epoch_manager,
        validator_node_client_factory,
        substate_store,
        template_manager,
    })
}

pub struct Services {
    pub keypair: RistrettoKeypair,
    pub networking: NetworkingHandle<TariMessagingSpec>,
    pub epoch_manager: EpochManagerHandle<PeerAddress>,
    pub validator_node_client_factory: TariValidatorNodeRpcClientFactory,
    pub substate_store: SqliteSubstateStore,
    pub template_manager: TemplateManager<PeerAddress>,
}

fn ensure_directories_exist(config: &ApplicationConfig) -> io::Result<()> {
    fs::create_dir_all(&config.indexer.data_dir)?;
    Ok(())
}

fn save_identities(config: &ApplicationConfig, identity: &RistrettoKeypair) -> Result<(), ExitError> {
    identity_management::save_as_json(&config.indexer.identity_file, identity)
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Failed to save node identity: {}", e)))?;

    Ok(())
}
