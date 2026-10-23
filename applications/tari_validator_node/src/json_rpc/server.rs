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

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Json,
    Router,
    extract::Extension,
    response::IntoResponse,
    routing::{get, post},
};
use axum_jrpc::{JrpcResult, JsonRpcAnswer, JsonRpcExtractor, error::JsonRpcErrorReason};
use log::*;
use serde_json::json;
use tari_consensus::hotstuff::ConsensusCurrentState;
use tari_ootle_app_utilities::tcp::try_bind_with_fallback;
use tari_shutdown::ShutdownSignal;
use tokio::task::JoinHandle;
use tower_http::cors::CorsLayer;

use super::handlers::JsonRpcHandlers;
use crate::http_ui;

const LOG_TARGET: &str = "tari::validator_node::json_rpc";

/// Spawn the JSON-RPC server. The returned [`JoinHandle`] completes when `shutdown_signal`
/// fires and axum's graceful shutdown has drained in-flight requests — this is how the
/// validator releases its final `Arc<TransactionDB>` clones at shutdown. Callers should
/// push the handle into `Services.handles` so `join_all` awaits it.
pub async fn spawn_json_rpc(
    preferred_address: SocketAddr,
    enable_permissive_cors: bool,
    handlers: JsonRpcHandlers,
    mut shutdown_signal: ShutdownSignal,
) -> Result<(SocketAddr, JoinHandle<Result<(), anyhow::Error>>), anyhow::Error> {
    let router = Router::new()
        .route("/", get(http_ui::handler).post(handler))
        .route("/json_rpc", post(handler))
        .route("/health", get(health_check))
        .fallback(http_ui::handler)
        .layer(Extension(Arc::new(handlers)))
        .layer(if enable_permissive_cors {
            CorsLayer::permissive()
        } else {
            CorsLayer::new()
        });

    let listener = try_bind_with_fallback(preferred_address).await?;
    let server = axum::serve(listener, router).with_graceful_shutdown(async move {
        shutdown_signal.wait().await;
    });
    let addr = server.local_addr()?;
    info!(target: LOG_TARGET, "🌐 JSON-RPC listening on {}", addr);
    let handle = tokio::spawn(async move {
        server.await.map_err(anyhow::Error::from)?;
        info!(target: LOG_TARGET, "💤 JSON-RPC server exited cleanly");
        Ok(())
    });

    Ok((addr, handle))
}

async fn handler(Extension(handlers): Extension<Arc<JsonRpcHandlers>>, value: JsonRpcExtractor) -> JrpcResult {
    debug!(target: LOG_TARGET, "🌐 JSON-RPC request: {}", value.method);
    let result = match value.method.as_str() {
        // Transaction
        // "get_transaction_status" => handlers.get_transaction_status(value).await,
        "submit_transaction" => handlers.submit_transaction(value).await,
        "get_transaction" => handlers.get_transaction(value).await,
        "get_transaction_result" => handlers.get_transaction_result(value).await,
        "get_state" => handlers.get_state(value).await,
        "get_substate" => handlers.get_substate(value).await,
        "list_blocks" => handlers.list_blocks(value).await,
        "get_tx_pool" => handlers.get_tx_pool(value).await,
        // Blocks
        "get_block" => handlers.get_block(value).await,
        "get_blocks" => handlers.get_blocks(value).await,
        "get_filtered_blocks_count" => handlers.get_filtered_blocks_count(value).await,
        // Template
        "get_template" => handlers.get_template(value).await,
        // Validator Node
        "get_identity" => handlers.get_identity(value).await,
        "get_mempool_stats" => handlers.get_mempool_stats(value).await,
        "get_epoch_manager_stats" => handlers.get_epoch_manager_stats(value).await,
        "get_shard_key" => handlers.get_shard_key(value).await,
        "get_committee" => handlers.get_committee(value).await,
        "get_all_vns" => handlers.get_all_vns(value).await,
        "get_consensus_status" => handlers.get_consensus_status(value).await,
        // Diagnostics
        "get_diagnostic_events" => handlers.get_diagnostic_events(value).await,
        "clear_diagnostic_events" => handlers.clear_diagnostic_events(value).await,
        // Operations
        "create_backup_snapshot" => handlers.create_backup_snapshot(value).await,
        // "get_network_committees" => handlers.get_network_committees(value).await,
        // Comms
        "add_peer" => handlers.add_peer(value).await,
        "get_comms_stats" => handlers.get_comms_stats(value).await,
        "get_connections" => handlers.get_connections(value).await,
        "prepare_layer_one_transaction" => handlers.prepare_layer_one_transaction(value).await,
        method => Ok(value.method_not_found(method)),
    };

    if let Err(ref e) = result {
        match &e.result {
            JsonRpcAnswer::Result(val) => {
                error!(
                    target: LOG_TARGET,
                    "🚨 JSON-RPC request failed: {}",
                    serde_json::to_string_pretty(val).unwrap_or_else(|e| e.to_string())
                );
            },
            // Log application errors as debug as these are typically intentional
            JsonRpcAnswer::Error(err) if matches!(err.error_reason(), JsonRpcErrorReason::ApplicationError(_)) => {
                debug!(target: LOG_TARGET, "JSON-RPC: {}", err);
            },
            JsonRpcAnswer::Error(err) => {
                error!(target: LOG_TARGET, "JSON-RPC request failed: {}", err);
            },
        }
    }
    result
}

async fn health_check(Extension(handlers): Extension<Arc<JsonRpcHandlers>>) -> impl IntoResponse {
    let is_net_ok = handlers.networking_is_active();
    let status = handlers.consensus_status();
    let is_ok = is_net_ok &&
        matches!(
            status,
            ConsensusCurrentState::Initialising |
                ConsensusCurrentState::Idle |
                ConsensusCurrentState::Running |
                ConsensusCurrentState::Syncing |
                ConsensusCurrentState::CheckSync
        );

    let body = json!({
        "status": if is_ok { "ok" } else { "error" },
        "networking_ok": is_net_ok,
        "consensus_status": status,
        "version": env!("CARGO_PKG_VERSION"),
    });

    if is_ok {
        (axum::http::StatusCode::OK, Json(body)).into_response()
    } else {
        (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
    }
}
