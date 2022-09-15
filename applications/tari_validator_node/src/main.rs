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
mod cmd_args;
mod comms;
mod config;
mod dan_node;
mod default_service_specification;
mod epoch_manager;
mod grpc;
mod json_rpc;
mod p2p;
mod template_manager;

use std::{process, sync::Arc};

use clap::Parser;
use log::*;
use tari_app_utilities::identity_management::setup_node_identity;
use tari_common::{
    exit_codes::{ExitCode, ExitError},
    initialize_logging,
    load_configuration,
};
use tari_comms::{peer_manager::PeerFeatures, NodeIdentity};
use tari_dan_core::storage::{global::GlobalDb, DbFactory};
use tari_dan_storage_sqlite::{global::SqliteGlobalDbBackendAdapter, SqliteDbFactory};
use tari_shutdown::{Shutdown, ShutdownSignal};
use template_manager::TemplateManager;
use tokio::{runtime, runtime::Runtime, task};

use crate::{
    bootstrap::spawn_services,
    cli::Cli,
    config::{ApplicationConfig, ValidatorNodeConfig},
    dan_node::DanNode,
    epoch_manager::EpochManager,
    grpc::services::base_node_client::GrpcBaseNodeClient,
    json_rpc::run_json_rpc,
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

async fn run_node(config: &ApplicationConfig) -> Result<(), ExitError> {
    let shutdown = Shutdown::new();

    let node_identity = setup_node_identity(
        &config.validator_node.identity_file,
        config.validator_node.public_address.as_ref(),
        true,
        PeerFeatures::NONE,
    )?;
    let db_factory = SqliteDbFactory::new(config.validator_node.data_dir.clone());
    let global_db = db_factory
        .get_or_create_global_db()
        .map_err(|e| ExitError::new(ExitCode::DatabaseError, e))?;

    info!(
        target: LOG_TARGET,
        "Node starting with pub key: {}, node_id: {}",
        node_identity.public_key(),
        node_identity.node_id()
    );
    // fs::create_dir_all(&global.peer_db_path).map_err(|err| ExitError::new(ExitCode::ConfigError, err))?;
    let _comms = spawn_services(config, shutdown.to_signal(), node_identity.clone()).await?;
    // let validator_node_client_factory =
    //     TariCommsValidatorNodeClientFactory::new();
    // let base_node_client = GrpcBaseNodeClient::new(config.validator_node.base_node_grpc_address);
    // let asset_proxy: ConcreteAssetProxy<DefaultServiceSpecification> = ConcreteAssetProxy::new(
    //     base_node_client.clone(),
    //     validator_node_client_factory,
    //     5,
    //     mempool_service.clone(),
    //     db_factory.clone(),
    // );
    // let grpc_server: ValidatorNodeGrpcServer<DefaultServiceSpecification> =
    //     ValidatorNodeGrpcServer::new(node_identity.as_ref().clone(), db_factory.clone(), asset_proxy);

    // Run the gRPC API
    // if let Some(address) = config.validator_node.grpc_address.clone() {
    //     println!("Started GRPC server on {}", address);
    //     task::spawn(run_grpc(grpc_server, address, shutdown.to_signal()));
    // }

    let epoch_manager = Arc::new(EpochManager::new());
    let template_manager = Arc::new(TemplateManager::new(db_factory.clone()));

    // Run the JSON-RPC API
    if let Some(address) = config.validator_node.json_rpc_address {
        println!("Started JSON-RPC server on {}", address);
        task::spawn(run_json_rpc(address, node_identity.as_ref().clone()));
    }

    // Show the validator node identity
    println!("🚀 Validator node started!");
    println!("{}", node_identity);

    run_dan_node(
        shutdown.to_signal(),
        config.validator_node.clone(),
        db_factory,
        node_identity,
        global_db,
        epoch_manager.clone(),
        template_manager.clone(),
    )
    .await?;

    Ok(())
}

fn build_runtime() -> Result<Runtime, ExitError> {
    let mut builder = runtime::Builder::new_multi_thread();
    builder
        .enable_all()
        .build()
        .map_err(|e| ExitError::new(ExitCode::UnknownError, e))
}

async fn run_dan_node(
    shutdown_signal: ShutdownSignal,
    config: ValidatorNodeConfig,
    db_factory: SqliteDbFactory,
    node_identity: Arc<NodeIdentity>,
    global_db: GlobalDb<SqliteGlobalDbBackendAdapter>,
    epoch_manager: Arc<EpochManager>,
    template_manager: Arc<TemplateManager>,
) -> Result<(), ExitError> {
    let node = DanNode::new(config, node_identity, global_db, epoch_manager, template_manager);
    node.start(shutdown_signal, db_factory).await
}

// async fn run_grpc<TServiceSpecification: ServiceSpecification + 'static>(
//     grpc_server: ValidatorNodeGrpcServer<TServiceSpecification>,
//     grpc_address: Multiaddr,
//     shutdown_signal: ShutdownSignal,
// ) -> Result<(), anyhow::Error> {
//     println!("Starting GRPC on {}", grpc_address);
//     info!(target: LOG_TARGET, "Starting GRPC on {}", grpc_address);
//
//     let grpc_address = multiaddr_to_socketaddr(&grpc_address)?;
//
//     Server::builder()
//         .add_service(ValidatorNodeServer::new(grpc_server))
//         .serve_with_shutdown(grpc_address, shutdown_signal.map(|_| ()))
//         .await
//         .map_err(|err| {
//             error!(target: LOG_TARGET, "GRPC encountered an error: {}", err);
//             err
//         })?;
//
//     info!("Stopping GRPC");
//     info!(target: LOG_TARGET, "Stopping GRPC");
//     Ok(())
// }
