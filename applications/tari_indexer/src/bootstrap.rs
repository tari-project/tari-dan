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

use std::{fs, io, sync::Arc};

use minotari_app_utilities::{identity_management, identity_management::load_from_json};
use tari_base_node_client::grpc::GrpcBaseNodeClient;
use tari_common::{
    configuration::bootstrap::{grpc_default_port, ApplicationType},
    exit_codes::{ExitCode, ExitError},
};
use tari_comms::{CommsNode, NodeIdentity};
use tari_dan_app_utilities::{
    base_layer_scanner,
    consensus_constants::ConsensusConstants,
    template_manager::{self, implementation::TemplateManager},
};
use tari_dan_storage::global::GlobalDb;
use tari_dan_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_epoch_manager::base_layer::{EpochManagerConfig, EpochManagerHandle};
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tari_validator_node_rpc::client::TariCommsValidatorNodeClientFactory;

use crate::{
    comms,
    p2p::services::{comms_peer_provider::CommsPeerProvider, networking},
    substate_storage_sqlite::sqlite_substate_store_factory::SqliteSubstateStore,
    ApplicationConfig,
};

const _LOG_TARGET: &str = "tari_indexer::bootstrap";

pub async fn spawn_services(
    config: &ApplicationConfig,
    shutdown: ShutdownSignal,
    node_identity: Arc<NodeIdentity>,
    global_db: GlobalDb<SqliteGlobalDbAdapter>,
    consensus_constants: ConsensusConstants,
) -> Result<Services, anyhow::Error> {
    let mut p2p_config = config.indexer.p2p.clone();
    p2p_config.transport.tor.identity =
        load_from_json(&config.indexer.tor_identity_file).map_err(|e| ExitError::new(ExitCode::ConfigError, e))?;
    ensure_directories_exist(config)?;

    // GRPC client connection to base node
    let base_node_client =
        GrpcBaseNodeClient::new(config.indexer.base_node_grpc_address.clone().unwrap_or_else(|| {
            let port = grpc_default_port(ApplicationType::BaseNode, config.network);
            format!("127.0.0.1:{port}")
        }));

    // Initialize comms
    let (comms, _) = comms::initialize(node_identity.clone(), config, shutdown.clone()).await?;

    networking::spawn(
        comms.node_identity(),
        CommsPeerProvider::new(comms.peer_manager()),
        comms.connectivity(),
    );
    // Connect to substate db
    let substate_store = SqliteSubstateStore::try_create(config.indexer.state_db_path())?;

    // Epoch manager
    let validator_node_client_factory = TariCommsValidatorNodeClientFactory::new(comms.connectivity());
    let (epoch_manager, _) = tari_epoch_manager::base_layer::spawn_service(
        EpochManagerConfig {
            base_layer_confirmations: consensus_constants.base_layer_confirmations,
            committee_size: consensus_constants.committee_size,
        },
        global_db.clone(),
        base_node_client.clone(),
        node_identity.public_key().clone(),
        shutdown.clone(),
    );

    // Template manager
    let template_manager = TemplateManager::initialize(global_db.clone(), config.indexer.templates.clone())?;
    let (template_manager_service, _) =
        template_manager::implementation::spawn(template_manager.clone(), shutdown.clone());

    // Base Node scanner
    base_layer_scanner::spawn(
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

    let comms = comms::spawn_comms_using_transport(comms, p2p_config.transport.clone())
        .await
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Could not spawn using transport: {}", e)))?;

    // Save final node identity after comms has initialized. This is required because the public_address can be
    // changed by comms during initialization when using tor.
    save_identities(config, &comms)?;
    Ok(Services {
        comms,
        epoch_manager,
        validator_node_client_factory,
        substate_store,
        template_manager,
    })
}

pub struct Services {
    pub comms: CommsNode,
    pub epoch_manager: EpochManagerHandle,
    pub validator_node_client_factory: TariCommsValidatorNodeClientFactory,
    pub substate_store: SqliteSubstateStore,
    pub template_manager: TemplateManager,
}

fn ensure_directories_exist(config: &ApplicationConfig) -> io::Result<()> {
    fs::create_dir_all(&config.indexer.data_dir)?;
    fs::create_dir_all(&config.indexer.p2p.datastore_path)?;
    Ok(())
}

fn save_identities(config: &ApplicationConfig, comms: &CommsNode) -> Result<(), ExitError> {
    identity_management::save_as_json(&config.indexer.identity_file, &*comms.node_identity())
        .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Failed to save node identity: {}", e)))?;

    if let Some(hs) = comms.hidden_service() {
        identity_management::save_as_json(&config.indexer.tor_identity_file, hs.tor_identity())
            .map_err(|e| ExitError::new(ExitCode::ConfigError, format!("Failed to save tor identity: {}", e)))?;
    }
    Ok(())
}
