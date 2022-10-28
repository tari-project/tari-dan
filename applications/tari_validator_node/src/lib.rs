// Copyright 2022. The Tari Project
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

mod base_layer_scanner;
mod bootstrap;
pub mod cli;
mod comms;
mod config;
mod consensus_constants;
mod dan_node;
mod default_service_specification;
mod grpc;
mod http_ui;
mod json_rpc;
mod p2p;
mod payload_processor;
mod registration;
mod template_registration_signing;
mod validator_node_registration_signing;

use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
};

use log::*;
use serde::{Deserialize, Serialize};
use tari_app_utilities::identity_management::setup_node_identity;
use tari_common::{
    configuration::bootstrap::{grpc_default_port, ApplicationType},
    exit_codes::{ExitCode, ExitError},
    initialize_logging,
};
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    services::{base_node_error::BaseNodeError, BaseNodeClient},
    storage::DbFactory,
    DigitalAssetError,
};
use tari_dan_storage_sqlite::SqliteDbFactory;
use tari_shutdown::{Shutdown, ShutdownSignal};
use tokio::task;

pub use crate::config::{ApplicationConfig, ValidatorNodeConfig};
use crate::{
    bootstrap::{spawn_services, Services},
    cli::Cli,
    consensus_constants::ConsensusConstants,
    dan_node::DanNode,
    grpc::services::{base_node_client::GrpcBaseNodeClient, wallet_client::GrpcWalletClient},
    http_ui::server::run_http_ui_server,
    json_rpc::{run_json_rpc, JsonRpcHandlers},
    p2p::services::networking::DAN_PEER_FEATURES,
};

const LOG_TARGET: &str = "tari::validator_node::app";

#[derive(Debug, thiserror::Error)]
pub enum ShardKeyError {
    #[error("Path is not a file")]
    NotFile,
    #[error("Malformed shard key file: {0}")]
    JsonError(#[from] json5::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Not yet mined")]
    NotYetMined,
    #[error("Not yet registered")]
    NotYetRegistered,
    #[error("Registration failed")]
    RegistrationFailed,
    #[error("Registration error {0}")]
    RegistrationError(#[from] DigitalAssetError),
    #[error("Base node error: {0}")]
    BaseNodeError(#[from] BaseNodeError),
}

#[derive(Serialize, Deserialize)]
pub struct ShardKey {
    is_registered: bool,
    shard_id: Option<ShardId>,
}

pub async fn run_validator_node_with_cli(config: &ApplicationConfig, cli: &Cli) -> Result<(), ExitError> {
    initialize_logging(
        &cli.common.log_config_path("validator"),
        include_str!("../log4rs_sample.yml"),
    )?;

    let shutdown = Shutdown::new();

    let node_identity = setup_node_identity(
        &config.validator_node.identity_file,
        config.validator_node.public_address.as_ref(),
        true,
        DAN_PEER_FEATURES,
    )?;
    let db_factory = SqliteDbFactory::new(config.validator_node.data_dir.clone());
    let global_db = db_factory
        .get_or_create_global_db()
        .map_err(|e| ExitError::new(ExitCode::DatabaseError, e))?;

    info!(
        target: LOG_TARGET,
        "🚀 Node starting with pub key: {}, address: {}",
        node_identity.public_key(),
        node_identity.public_address()
    );

    let (base_node_client, wallet_client) = create_base_layer_clients(config).await?;

    let services = spawn_services(
        config,
        shutdown.to_signal(),
        node_identity.clone(),
        global_db,
        db_factory,
        ConsensusConstants::devnet(), // TODO: change this eventually
    )
    .await?;

    // Run the JSON-RPC API
    if let Some(address) = config.validator_node.json_rpc_address {
        info!(target: LOG_TARGET, "🌐 Started JSON-RPC server on {}", address);
        let handlers = JsonRpcHandlers::new(wallet_client, base_node_client, &services);
        task::spawn(run_json_rpc(address, handlers));
    }

    // Run the http ui
    if let Some(address) = config.validator_node.http_ui_address {
        info!(target: LOG_TARGET, "🕸️ Started HTTP UI server on {}", address);
        task::spawn(run_http_ui_server(
            address,
            config.validator_node.json_rpc_address.map(|addr| addr.to_string()),
        ));
    }

    run_dan_node(services, shutdown.to_signal()).await?;

    Ok(())
}

async fn run_dan_node(services: Services, shutdown_signal: ShutdownSignal) -> Result<(), ExitError> {
    let node = DanNode::new(services);
    info!(target: LOG_TARGET, "🚀 Validator node started!");
    node.start(shutdown_signal).await
}

async fn create_base_layer_clients(
    config: &ApplicationConfig,
) -> Result<(GrpcBaseNodeClient, GrpcWalletClient), ExitError> {
    let mut base_node_client =
        GrpcBaseNodeClient::new(config.validator_node.base_node_grpc_address.unwrap_or_else(|| {
            let port = grpc_default_port(ApplicationType::BaseNode, config.network);
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
        }));
    base_node_client
        .test_connection()
        .await
        .map_err(|error| ExitError::new(ExitCode::ConfigError, error))?;

    let wallet_client = GrpcWalletClient::new(config.validator_node.wallet_grpc_address.unwrap_or_else(|| {
        let port = grpc_default_port(ApplicationType::ConsoleWallet, config.network);
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }));

    Ok((base_node_client, wallet_client))
}
