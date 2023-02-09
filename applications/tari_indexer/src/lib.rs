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

mod bootstrap;
pub mod cli;
mod comms;
mod grpc;
mod json_rpc;
mod p2p;

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    panic,
};

use cli::Cli;
use log::*;
use tari_app_utilities::identity_management::setup_node_identity;
use tari_common::{
    configuration::bootstrap::{grpc_default_port, ApplicationType},
    exit_codes::{ExitCode, ExitError},
};
use tari_comms::peer_manager::PeerFeatures;
use tari_dan_core::{consensus_constants::ConsensusConstants, services::BaseNodeClient, storage::DbFactory};
use tari_dan_storage_sqlite::SqliteDbFactory;
use tari_engine_types::substate::SubstateAddress;
use tari_shutdown::ShutdownSignal;
use tari_validator_node::ApplicationConfig;
use tokio::{task, time, time::Duration};

use crate::{
    bootstrap::spawn_services,
    grpc::services::base_node_client::GrpcBaseNodeClient,
    json_rpc::{run_json_rpc, JsonRpcHandlers},
};

const LOG_TARGET: &str = "tari::indexer::app";
const DEFAULT_POLL_TIME_MS: u64 = 200;
pub const DAN_PEER_FEATURES: PeerFeatures = PeerFeatures::COMMUNICATION_NODE;

pub async fn run_indexer(
    cli: Cli,
    config: ApplicationConfig,
    mut shutdown_signal: ShutdownSignal,
) -> Result<(), ExitError> {
    let node_identity = setup_node_identity(
        &config.validator_node.identity_file,
        config.validator_node.public_address.as_ref(),
        true,
        DAN_PEER_FEATURES,
    )?;

    let db_factory = SqliteDbFactory::new(config.validator_node.data_dir.clone());
    db_factory
        .migrate()
        .map_err(|e| ExitError::new(ExitCode::DatabaseError, e))?;
    let global_db = db_factory
        .get_or_create_global_db()
        .map_err(|e| ExitError::new(ExitCode::DatabaseError, e))?;

    let _base_node_client = create_base_layer_clients(&config).await?;
    let _services = spawn_services(
        &config,
        shutdown_signal.clone(),
        node_identity.clone(),
        global_db,
        ConsensusConstants::devnet(), // TODO: change this eventually
    )
    .await?;

    // Run the JSON-RPC API
    if let Some(json_rpc_address) = cli.json_rpc_address {
        info!(target: LOG_TARGET, "🌐 Started JSON-RPC server on {}", json_rpc_address);
        let handlers = JsonRpcHandlers::new(cli.address.clone());
        task::spawn(run_json_rpc(json_rpc_address, handlers));
    }

    let poll_time_ms = cli.poll_time_ms.unwrap_or(DEFAULT_POLL_TIME_MS);
    loop {
        tokio::select! {
            _ = time::sleep(Duration::from_millis(poll_time_ms)) => {
                scan_substates(&cli.address).await;
            },

            _ = shutdown_signal.wait() => {
                break;
            },
        }
    }

    Ok(())
}

async fn scan_substates(_addresses: &[SubstateAddress]) {
    // TODO
}

async fn create_base_layer_clients(config: &ApplicationConfig) -> Result<GrpcBaseNodeClient, ExitError> {
    let mut base_node_client =
        GrpcBaseNodeClient::new(config.validator_node.base_node_grpc_address.unwrap_or_else(|| {
            let port = grpc_default_port(ApplicationType::BaseNode, config.network);
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
        }));
    base_node_client
        .test_connection()
        .await
        .map_err(|error| ExitError::new(ExitCode::ConfigError, error))?;

    Ok(base_node_client)
}
