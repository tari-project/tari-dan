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

mod bootstrap;
pub mod cli;
mod config;
mod consensus;
mod dan_node;
mod dry_run_transaction_processor;
mod event_subscription;
mod grpc;
mod http_ui;
mod json_rpc;
mod p2p;
mod registration;
mod substate_resolver;
mod template_registration_signing;
mod virtual_substate;

use std::{
    collections::HashMap,
    fs,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    process,
};

use log::*;
use serde::{Deserialize, Serialize};
use tari_base_node_client::{grpc::GrpcBaseNodeClient, BaseNodeClientError};
use tari_common::{
    configuration::bootstrap::{grpc_default_port, ApplicationType},
    exit_codes::{ExitCode, ExitError},
};
use tari_common_types::types::PublicKey;
use tari_dan_app_utilities::{consensus_constants::ConsensusConstants, keypair::setup_keypair_prompt};
use tari_dan_common_types::SubstateAddress;
use tari_dan_storage::global::DbFactory;
use tari_dan_storage_sqlite::SqliteDbFactory;
use tari_shutdown::ShutdownSignal;
use tokio::task;

pub use crate::config::{ApplicationConfig, ValidatorNodeConfig};
use crate::{
    bootstrap::{spawn_services, Services},
    dan_node::DanNode,
    grpc::base_layer_wallet::{GrpcWalletClient, WalletGrpcError},
    http_ui::server::run_http_ui_server,
    json_rpc::{spawn_json_rpc, JsonRpcHandlers},
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
    RegistrationError(#[from] WalletGrpcError),
    #[error("Base node error: {0}")]
    BaseNodeError(#[from] BaseNodeClientError),
}

#[derive(Serialize, Deserialize)]
pub struct ShardKey {
    is_registered: bool,
    substate_address: Option<SubstateAddress>,
}

pub async fn run_validator_node(config: &ApplicationConfig, shutdown_signal: ShutdownSignal) -> Result<(), ExitError> {
    let keypair = setup_keypair_prompt(
        &config.validator_node.identity_file,
        !config.validator_node.dont_create_id,
    )?;

    let db_factory = SqliteDbFactory::new(config.validator_node.data_dir.clone());
    db_factory
        .migrate()
        .map_err(|e| ExitError::new(ExitCode::DatabaseError, e))?;
    let global_db = db_factory
        .get_or_create_global_db()
        .map_err(|e| ExitError::new(ExitCode::DatabaseError, e))?;

    info!(
        target: LOG_TARGET,
        "🚀 Node starting with pub key: {} and peer id {}",
        keypair.public_key(),keypair.to_peer_address(),
    );

    let metrics_registry = create_metrics_registry(keypair.public_key());

    let (base_node_client, wallet_client) = create_base_layer_clients(config).await?;
    let services = spawn_services(
        config,
        shutdown_signal.clone(),
        keypair.clone(),
        global_db,
        ConsensusConstants::devnet(), // TODO: change this eventually
        &metrics_registry,
    )
    .await?;
    let info = services.networking.get_local_peer_info().await.unwrap();
    info!(target: LOG_TARGET, "🚀 Node started: {}", info);

    // Run the JSON-RPC API
    let mut jrpc_address = config.validator_node.json_rpc_listener_address;
    if let Some(jrpc_address) = jrpc_address.as_mut() {
        info!(target: LOG_TARGET, "🌐 Started JSON-RPC server on {}", jrpc_address);
        let handlers = JsonRpcHandlers::new(wallet_client, base_node_client, &services);
        *jrpc_address = spawn_json_rpc(*jrpc_address, handlers, metrics_registry)?;
        // Run the http ui
        if let Some(address) = config.validator_node.http_ui_listener_address {
            task::spawn(run_http_ui_server(
                address,
                config
                    .validator_node
                    .json_rpc_public_address
                    .clone()
                    .unwrap_or_else(|| jrpc_address.to_string()),
            ));
        }
    }

    fs::write(config.common.base_path.join("pid"), process::id().to_string())
        .map_err(|e| ExitError::new(ExitCode::UnknownError, e))?;
    run_dan_node(services, shutdown_signal).await?;

    Ok(())
}

async fn run_dan_node(services: Services, shutdown_signal: ShutdownSignal) -> Result<(), ExitError> {
    let node = DanNode::new(services);
    info!(target: LOG_TARGET, "🚀 Validator node started!");
    node.start(shutdown_signal)
        .await
        .map_err(|e| ExitError::new(ExitCode::UnknownError, e))
}

async fn create_base_layer_clients(
    config: &ApplicationConfig,
) -> Result<(GrpcBaseNodeClient, GrpcWalletClient), ExitError> {
    let base_node_client =
        GrpcBaseNodeClient::connect(config.validator_node.base_node_grpc_address.clone().unwrap_or_else(|| {
            let port = grpc_default_port(ApplicationType::BaseNode, config.network);
            format!("127.0.0.1:{port}")
        }))
        .await
        .map_err(|error| ExitError::new(ExitCode::ConfigError, error))?;

    let wallet_client = GrpcWalletClient::new(config.validator_node.wallet_grpc_address.unwrap_or_else(|| {
        let port = grpc_default_port(ApplicationType::ConsoleWallet, config.network);
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }));

    Ok((base_node_client, wallet_client))
}

fn create_metrics_registry(public_key: &PublicKey) -> prometheus::Registry {
    let mut labels = HashMap::with_capacity(2);
    labels.insert("app".to_string(), "ValidatorNode".to_string());
    labels.insert("public_key".to_string(), public_key.to_string());
    prometheus::Registry::new_custom(Some("tari".to_string()), Some(labels)).unwrap()
}
