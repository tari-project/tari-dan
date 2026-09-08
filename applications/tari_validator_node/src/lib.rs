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

mod base_layer;
mod bootstrap;
mod config;
pub mod consensus;
#[cfg(feature = "metrics")]
mod epoch_metrics;
mod event_subscription;
mod file_l1_submitter;
mod genesis_state;
#[cfg(feature = "web_ui")]
mod http_ui;
#[cfg(feature = "metrics")]
mod inbound_queue_metrics;
mod json_rpc;
mod memory_budget;
#[cfg(feature = "metrics")]
mod metrics;
mod migrations;
pub mod node;
mod p2p;
#[cfg(feature = "metrics")]
mod state_store_metrics;

use std::{fs, io, iter, process, time::Instant};

use log::*;
use serde::{Deserialize, Serialize};
use tari_common::exit_codes::{ExitCode, ExitError};
use tari_engine_types::crypto::{MAX_LAZY_BP_AGG_FACTORS, get_commitment_factory, get_static_range_proof_service};
use tari_epoch_manager::traits::EpochManagerSpec;
use tari_epoch_oracles::EpochOracle;
use tari_ootle_app_utilities::{
    consensus_constants_file::load_consensus_constants,
    keypair::RistrettoKeypair,
    protocol_activation::check_and_record_activation_schedule,
};
use tari_ootle_common_types::SubstateAddress;
use tari_ootle_p2p::PeerAddress;
use tari_ootle_storage::global::{DbFactory, GlobalDb};
use tari_ootle_storage_sqlite::{SqliteDbFactory, global::SqliteGlobalDbAdapter};
use tari_shutdown::Shutdown;
use tokio::task;

pub use crate::config::{ApplicationConfig, ValidatorNodeConfig};
use crate::{
    bootstrap::{Services, spawn_services},
    consensus::spec::ValidatorNodeStateStore,
    file_l1_submitter::FileLayerOneSubmitter,
    json_rpc::{JsonRpcHandlers, spawn_json_rpc},
    node::ValidatorNode,
};

const LOG_TARGET: &str = "tari::validator_node::app";

#[derive(Debug, thiserror::Error)]
pub enum ShardKeyError {
    #[error("Path is not a file")]
    NotFile,
    #[error("Malformed shard key file: {0}")]
    JsonError(#[from] serde_json5::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Not yet mined")]
    NotYetMined,
    #[error("Not yet registered")]
    NotYetRegistered,
    #[error("Registration failed")]
    RegistrationFailed,
}

#[derive(Serialize, Deserialize)]
pub struct ShardKey {
    is_registered: bool,
    substate_address: Option<SubstateAddress>,
}

pub async fn run_validator_node(
    keypair: RistrettoKeypair,
    config: ApplicationConfig,
    shutdown: Shutdown,
) -> Result<(), anyhow::Error> {
    info!(target: LOG_TARGET, "Starting validator node on network {}", config.network);

    let db_factory = SqliteDbFactory::new(config.validator_node.get_global_db_path());
    db_factory
        .migrate()
        .map_err(|e| ExitError::new(ExitCode::DatabaseError, e))?;
    let global_db = db_factory
        .get_or_create_global_db()
        .map_err(|e| ExitError::new(ExitCode::DatabaseError, e))?;

    check_and_record_activation_schedule(
        config.network,
        &global_db,
        config.validator_node.allow_past_protocol_activation,
    )?;

    info!(
        target: LOG_TARGET,
        "🚀 Node starting with pub key: {} and peer id {}",
        keypair.public_key(),keypair.to_peer_address(),
    );

    // Preload the range proof services. This avoids initialization cost during transaction processing and helps ensure
    // validators execute at a more similar speed.
    task::spawn_blocking(preload_crypto_services).await?;

    #[cfg(feature = "metrics")]
    let mut base_registry = prometheus_client::registry::Registry::default();

    #[cfg(feature = "metrics")]
    let metrics_registry = create_metrics_registry(keypair.public_key(), &mut base_registry);

    let consensus_constants = load_consensus_constants(
        config.network,
        config.validator_node.localnet_consensus_constants_file.as_deref(),
        &config.validator_node.default_localnet_consensus_constants_file(),
    )?;
    let mut services = spawn_services(
        config.clone(),
        shutdown.to_signal(),
        keypair.clone(),
        global_db,
        consensus_constants,
        #[cfg(feature = "metrics")]
        metrics_registry,
    )
    .await?;
    let info = services.networking.get_local_peer_info().await?;
    info!(target: LOG_TARGET, "🚀 Node started: {}", info);

    // Run the JSON-RPC API
    let mut jrpc_address = config.validator_node.json_rpc_listener_address;
    if let Some(jrpc_address) = jrpc_address.as_mut() {
        info!(target: LOG_TARGET, "🌐 Started JSON-RPC server on {}", jrpc_address);
        let handlers = JsonRpcHandlers::new(&services);
        let (bound_addr, jrpc_handle) = spawn_json_rpc(
            *jrpc_address,
            handlers,
            shutdown.to_signal(),
            #[cfg(feature = "metrics")]
            base_registry,
        )
        .await?;
        *jrpc_address = bound_addr;
        // Track the axum handle so `Services::join_all` awaits it at shutdown — this is
        // what guarantees the final `Arc<TransactionDB>` clone held by the handlers is
        // dropped before `run_validator_node` returns.
        services.handles.push(jrpc_handle);

        // Run the web ui
        #[cfg(feature = "web_ui")]
        if let Some(address) = config.validator_node.web_ui_listener_address {
            tokio::spawn(http_ui::server::run_http_ui_server(
                address,
                config
                    .validator_node
                    .json_rpc_public_url
                    .clone()
                    .unwrap_or_else(|| jrpc_address.to_string()),
            ));
        }
    }
    #[cfg(not(feature = "web_ui"))]
    info!(
        target: LOG_TARGET,
        "Web UI is not enabled. To enable it, add the `web_ui` feature to your Cargo.toml"
    );

    fs::write(config.common.base_path.join("pid"), process::id().to_string())
        .map_err(|e| ExitError::new(ExitCode::UnknownError, e))?;
    let node = ValidatorNode::new(services);
    info!(target: LOG_TARGET, "🚀 Validator node started!");
    node.start(shutdown)
        .await
        .map_err(|e| ExitError::new(ExitCode::UnknownError, e))?;

    Ok(())
}

#[cfg(feature = "metrics")]
fn create_metrics_registry<'a>(
    public_key: &tari_crypto::ristretto::RistrettoPublicKey,
    registry_mut: &'a mut prometheus_client::registry::Registry,
) -> &'a mut prometheus_client::registry::Registry {
    use std::borrow::Cow;
    registry_mut.sub_registry_with_labels(
        [
            (Cow::Borrowed("app"), Cow::Borrowed("ValidatorNode")),
            (Cow::Borrowed("public_key"), Cow::Owned(public_key.to_string())),
        ]
        .into_iter(),
    )
}

fn preload_crypto_services() {
    let timer = Instant::now();
    // iterate through the power of 2 up to the max aggregation factor
    let agg_factors = iter::successors(Some(1usize), |po2| {
        let next = (po2 + 1).next_power_of_two();
        if next > MAX_LAZY_BP_AGG_FACTORS {
            None
        } else {
            Some(next)
        }
    });
    for agg in agg_factors {
        // Preload the static range proof services into memory so that initialization cost is not paid during
        // execution
        let _ = get_static_range_proof_service(agg);
    }
    let _ = get_commitment_factory();

    info!(
        target: LOG_TARGET,
        "Preloaded crypto services in {:.2?}",
        timer.elapsed()
    );
}

pub struct ValidatorNodeEpochManagerSpec;

impl EpochManagerSpec for ValidatorNodeEpochManagerSpec {
    type Addr = PeerAddress;
    // When metrics are enabled, the oracle is wrapped by MeteredEpochOracle so it records every
    // event it forwards to the epoch manager. When disabled, the raw oracle is used directly.
    #[cfg(not(feature = "metrics"))]
    type EpochEventOracle = EpochOracle<GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>>;
    #[cfg(feature = "metrics")]
    type EpochEventOracle =
        crate::epoch_metrics::MeteredEpochOracle<EpochOracle<GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>>>;
    type LayerOneSubmitter = FileLayerOneSubmitter;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preload_crypto_services_does_not_panic() {
        preload_crypto_services();
        preload_crypto_services();
    }
}
