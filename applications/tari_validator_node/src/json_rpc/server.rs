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

use axum::{extract::Extension, routing::post, Router};
use axum_jrpc::{JrpcResult, JsonRpcExtractor};
use log::*;
use tower_http::cors::CorsLayer;

use super::handlers::JsonRpcHandlers;

const LOG_TARGET: &str = "tari::validator_node::json_rpc";

pub fn spawn_json_rpc(preferred_address: SocketAddr, handlers: JsonRpcHandlers) -> Result<SocketAddr, anyhow::Error> {
    let router = Router::new()
        .route("/", post(handler))
        .route("/json_rpc", post(handler))
        .layer(Extension(Arc::new(handlers)))
        .layer(CorsLayer::permissive());

    let server = axum::Server::try_bind(&preferred_address).or_else(|_| {
        error!(
            target: LOG_TARGET,
            "🌐 Failed to bind on preferred address {}. Trying OS-assigned", preferred_address
        );
        axum::Server::try_bind(&"127.0.0.1:0".parse().unwrap())
    })?;
    let server = server.serve(router.into_make_service());
    let addr = server.local_addr();
    info!(target: LOG_TARGET, "🌐 JSON-RPC listening on {}", addr);
    tokio::spawn(server);

    info!(target: LOG_TARGET, "💤 Stopping JSON-RPC");
    Ok(addr)
}

async fn handler(Extension(handlers): Extension<Arc<JsonRpcHandlers>>, value: JsonRpcExtractor) -> JrpcResult {
    debug!(target: LOG_TARGET, "🌐 JSON-RPC request: {}", value.method);
    match value.method.as_str() {
        // Transaction
        // "get_transaction_status" => handlers.get_transaction_status(value).await,
        "submit_transaction" => handlers.submit_transaction(value).await,
        "get_recent_transactions" => handlers.get_recent_transactions(value).await,
        "get_transaction" => handlers.get_transaction(value).await,
        "get_transaction_result" => handlers.get_transaction_result(value).await,
        "get_transaction_qcs" => handlers.get_transaction_qcs(value).await,
        "get_state" => handlers.get_state(value).await,
        "get_substate" => handlers.get_substate(value).await,
        "get_substates" => handlers.get_substates(value).await,
        "get_current_leader_state" => handlers.get_current_leader_state(value).await,
        // Template
        "get_template" => handlers.get_template(value).await,
        "get_templates" => handlers.get_templates(value).await,
        "register_template" => handlers.register_template(value).await,
        // Validator Node
        "get_identity" => handlers.get_identity(value),
        "register_validator_node" => handlers.register_validator_node(value).await,
        "get_mempool_stats" => handlers.get_mempool_stats(value).await,
        "get_epoch_manager_stats" => handlers.get_epoch_manager_stats(value).await,
        "get_shard_key" => handlers.get_shard_key(value).await,
        "get_committee" => handlers.get_committee(value).await,
        "get_all_vns" => handlers.get_all_vns(value).await,
        "get_fees" => handlers.get_fees(value).await,
        // Comms
        "add_peer" => handlers.add_peer(value).await,
        "get_comms_stats" => handlers.get_comms_stats(value).await,
        "get_connections" => handlers.get_connections(value).await,
        // Debug
        "get_logged_messages" => handlers.get_logged_messages(value).await,
        method => Ok(value.method_not_found(method)),
    }
}
