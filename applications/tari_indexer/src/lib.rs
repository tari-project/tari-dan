// Copyright 2023. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_migrations;

mod bootstrap;
pub mod cli;
mod comms;
pub mod config;
mod dry_run;
pub mod graphql;
mod http_ui;

mod json_rpc;
mod p2p;
mod substate_manager;
mod substate_storage_sqlite;
mod transaction_manager;

use std::{fs, sync::Arc};

use http_ui::server::run_http_ui_server;
use log::*;
use minotari_app_utilities::identity_management::setup_node_identity;
use substate_manager::SubstateManager;
use tari_base_node_client::grpc::GrpcBaseNodeClient;
use tari_common::{
    configuration::bootstrap::{grpc_default_port, ApplicationType},
    exit_codes::{ExitCode, ExitError},
};
use tari_comms::peer_manager::PeerFeatures;
use tari_dan_app_utilities::consensus_constants::ConsensusConstants;
use tari_dan_storage::global::DbFactory;
use tari_dan_storage_sqlite::SqliteDbFactory;
use tari_indexer_lib::substate_scanner::SubstateScanner;
use tari_shutdown::ShutdownSignal;
use tokio::{task, time};

use crate::{
    bootstrap::{spawn_services, Services},
    config::ApplicationConfig,
    dry_run::processor::DryRunTransactionProcessor,
    graphql::server::run_graphql,
    json_rpc::{run_json_rpc, JsonRpcHandlers},
    transaction_manager::TransactionManager,
};

const LOG_TARGET: &str = "tari::indexer::app";
pub const DAN_PEER_FEATURES: PeerFeatures = PeerFeatures::COMMUNICATION_NODE;

pub async fn run_indexer(config: ApplicationConfig, mut shutdown_signal: ShutdownSignal) -> Result<(), ExitError> {
    let node_identity = setup_node_identity(
        &config.indexer.identity_file,
        config.indexer.p2p.public_addresses.to_vec(),
        true,
        DAN_PEER_FEATURES,
    )?;

    let db_factory = SqliteDbFactory::new(config.indexer.data_dir.clone());
    db_factory
        .migrate()
        .map_err(|e| ExitError::new(ExitCode::DatabaseError, e))?;
    let global_db = db_factory
        .get_or_create_global_db()
        .map_err(|e| ExitError::new(ExitCode::DatabaseError, e))?;

    let base_node_client = create_base_layer_clients(&config).await?;
    let services: Services = spawn_services(
        &config,
        shutdown_signal.clone(),
        node_identity.clone(),
        global_db,
        ConsensusConstants::devnet(), // TODO: change this eventually
    )
    .await?;

    let dan_layer_scanner = Arc::new(SubstateScanner::new(
        services.epoch_manager.clone(),
        services.validator_node_client_factory.clone(),
    ));

    let substate_manager = Arc::new(SubstateManager::new(
        dan_layer_scanner.clone(),
        services.substate_store.clone(),
    ));
    let transaction_manager = TransactionManager::new(
        services.epoch_manager.clone(),
        services.validator_node_client_factory.clone(),
        dan_layer_scanner.clone(),
    );

    // dry run
    let dry_run_transaction_processor = DryRunTransactionProcessor::new(
        services.epoch_manager.clone(),
        services.validator_node_client_factory.clone(),
        dan_layer_scanner,
        services.template_manager.clone(),
    );

    // Run the JSON-RPC API
    let jrpc_address = config.indexer.json_rpc_address;
    if let Some(address) = jrpc_address {
        info!(target: LOG_TARGET, "🌐 Started JSON-RPC server on {}", address);
        let consensus_constants = services
            .epoch_manager
            .get_base_layer_consensus_constants()
            .await
            .map_err(|e| ExitError::new(ExitCode::UnknownError, e))?;
        let handlers = JsonRpcHandlers::new(
            consensus_constants,
            &services,
            base_node_client,
            substate_manager.clone(),
            transaction_manager,
            dry_run_transaction_processor,
        );
        task::spawn(run_json_rpc(address, handlers));
        // Run the http ui
        if let Some(address) = config.indexer.http_ui_address {
            task::spawn(run_http_ui_server(
                address,
                config.indexer.ui_connect_address.unwrap_or(address.to_string()),
            ));
        }
    }
    // Run the GraphQL API
    let graphql_address = config.indexer.graphql_address;
    if let Some(address) = graphql_address {
        info!(target: LOG_TARGET, "🌐 Started GraphQL server on {}", address);
        task::spawn(run_graphql(address, substate_manager.clone()));
    }

    // Create pid to allow watchers to know that the process has started
    fs::write(config.common.base_path.join("pid"), std::process::id().to_string())
        .map_err(|e| ExitError::new(ExitCode::IOError, e))?;

    // keep scanning the dan layer for new versions of the stored substates
    loop {
        tokio::select! {
            _ = time::sleep(config.indexer.dan_layer_scanning_internal) => {
                match substate_manager.scan_and_update_substates().await {
                    Ok(_) => info!(target: LOG_TARGET, "Substate auto-scan succeded"),
                    Err(e) =>  error!(target: LOG_TARGET, "Substate auto-scan failed: {}", e),
                }
            },
            _ = shutdown_signal.wait() => {
                dbg!("Shutting down run_substate_polling");
                break;
            },
        }
    }

    shutdown_signal.wait().await;

    Ok(())
}

async fn create_base_layer_clients(config: &ApplicationConfig) -> Result<GrpcBaseNodeClient, ExitError> {
    GrpcBaseNodeClient::connect(config.indexer.base_node_grpc_address.clone().unwrap_or_else(|| {
        let port = grpc_default_port(ApplicationType::BaseNode, config.network);
        format!("127.0.0.1:{port}")
    }))
    .await
    .map_err(|err| ExitError::new(ExitCode::ConfigError, format!("Could not connect to base node {}", err)))
}
