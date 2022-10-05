// Copyright 2021. The Tari Project
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
mod cli;
mod comms;
mod config;
mod dan_node;
mod default_service_specification;
mod grpc;
mod http_ui;
mod json_rpc;
mod p2p;
mod payload_processor;
mod template_registration_signing;
mod validator_node_registration_signing;

use std::{fs, io, process};

use clap::Parser;
use log::*;
use serde::{Deserialize, Serialize};
use tari_app_utilities::identity_management::setup_node_identity;
use tari_common::{
    exit_codes::{ExitCode, ExitError},
    initialize_logging,
    load_configuration,
};
use tari_comms::NodeIdentity;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    services::{base_node_error::BaseNodeError, BaseNodeClient},
    storage::DbFactory,
    DigitalAssetError,
};
use tari_dan_storage_sqlite::SqliteDbFactory;
use tari_shutdown::{Shutdown, ShutdownSignal};
use tokio::{runtime, runtime::Runtime, task};

use crate::{
    bootstrap::{spawn_services, Services},
    cli::Cli,
    config::{ApplicationConfig, ValidatorNodeConfig},
    dan_node::DanNode,
    grpc::services::{base_node_client::GrpcBaseNodeClient, wallet_client::GrpcWalletClient},
    http_ui::server::run_http_ui_server,
    json_rpc::{run_json_rpc, JsonRpcHandlers},
    p2p::services::networking::DAN_PEER_FEATURES,
};

const LOG_TARGET: &str = "tari::validator_node::app";

fn main() {
    // Uncomment to enable tokio tracing via tokio-console
    // console_subscriber::init();

    if let Err(err) = main_inner() {
        let exit_code = err.exit_code;
        eprintln!("{:?}", err);
        error!(
            target: LOG_TARGET,
            "Exiting with code ({}): {:?}", exit_code as i32, exit_code
        );
        process::exit(exit_code as i32);
    }
}

fn main_inner() -> Result<(), ExitError> {
    let cli = Cli::parse();
    let config_path = cli.common.config_path();
    let cfg = load_configuration(config_path, true, &cli)?;
    initialize_logging(
        &cli.common.log_config_path("validator"),
        include_str!("../log4rs_sample.yml"),
    )?;
    let config = ApplicationConfig::load_from(&cfg)?;
    println!("Starting validator node on network {}", config.network);
    let runtime = build_runtime()?;
    runtime.block_on(run_node(&config))?;

    Ok(())
}

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

async fn auto_register_vn(
    wallet_client: &mut GrpcWalletClient,
    base_node_client: &mut GrpcBaseNodeClient,
    node_identity: &NodeIdentity,
    config: &ApplicationConfig,
) -> Result<ShardId, ShardKeyError> {
    let path = &config.validator_node.shard_key_file;

    // We already sent the registration tx, we are just waiting for it to be mined.
    let tip = base_node_client.get_tip_info().await?.height_of_longest_chain;
    let shard_id = base_node_client
        .get_shard_key(tip, node_identity.public_key())
        .await
        .map_err(ShardKeyError::BaseNodeError)?;
    if let Some(shard_id) = shard_id {
        let shard_key = ShardKey {
            is_registered: true,
            shard_id: Some(shard_id),
        };
        let json = json5::to_string(&shard_key)?;
        fs::write(path, json.as_bytes())?;
        Ok(shard_id)
    } else {
        let vn = wallet_client.register_validator_node(node_identity).await?;
        if vn.is_success {
            println!("Registering VN was successful {:?}", vn);
            let shard_key = ShardKey {
                is_registered: true,
                shard_id: None,
            };
            let json = json5::to_string(&shard_key)?;
            if let Some(p) = path.parent() {
                if !p.exists() {
                    fs::create_dir_all(p)?;
                }
            }
            fs::write(path, json.as_bytes())?;
            Err(ShardKeyError::NotYetRegistered)
        } else {
            Err(ShardKeyError::RegistrationFailed)
        }
    }
}

async fn run_node(config: &ApplicationConfig) -> Result<(), ExitError> {
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

    // fs::create_dir_all(&global.peer_db_path).map_err(|err| ExitError::new(ExitCode::ConfigError, err))?;
    let mut base_node_client = GrpcBaseNodeClient::new(config.validator_node.base_node_grpc_address);
    let mut wallet_client = GrpcWalletClient::new(config.validator_node.wallet_grpc_address);
    let vn_registration = auto_register_vn(&mut wallet_client, &mut base_node_client, &node_identity, config).await;
    println!("VN Registration result : {:?}", vn_registration);
    let services = spawn_services(
        config,
        shutdown.to_signal(),
        node_identity.clone(),
        global_db,
        db_factory,
    )
    .await?;

    // Run the JSON-RPC API
    if let Some(address) = config.validator_node.json_rpc_address {
        info!(target: LOG_TARGET, "🌐 Started JSON-RPC server on {}", address);
        let handlers = JsonRpcHandlers::new(
            GrpcWalletClient::new(config.validator_node.wallet_grpc_address),
            base_node_client,
            &services,
        );
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

    // Show the validator node identity
    info!(target: LOG_TARGET, "🚀 Validator node started!");

    run_dan_node(services, shutdown.to_signal()).await?;

    Ok(())
}

fn build_runtime() -> Result<Runtime, ExitError> {
    let mut builder = runtime::Builder::new_multi_thread();
    builder
        .enable_all()
        .build()
        .map_err(|e| ExitError::new(ExitCode::UnknownError, e))
}

async fn run_dan_node(services: Services, shutdown_signal: ShutdownSignal) -> Result<(), ExitError> {
    let node = DanNode::new(services);
    node.start(shutdown_signal).await
}
