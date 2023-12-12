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

use std::{net::SocketAddr, sync::Arc};

use axum::{extract::Extension, routing::post, Router};
use axum_jrpc::{JrpcResult, JsonRpcExtractor};
use log::*;
use tower_http::cors::CorsLayer;

use super::handlers::JsonRpcHandlers;

const LOG_TARGET: &str = "tari::indexer::json_rpc";

pub async fn run_json_rpc(preferred_address: SocketAddr, handlers: JsonRpcHandlers) -> Result<(), anyhow::Error> {
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
    info!(target: LOG_TARGET, "🌐 JSON-RPC listening on {}", server.local_addr());
    server.await?;

    info!(target: LOG_TARGET, "💤 Stopping JSON-RPC");
    Ok(())
}

async fn handler(Extension(handlers): Extension<Arc<JsonRpcHandlers>>, value: JsonRpcExtractor) -> JrpcResult {
    debug!(target: LOG_TARGET, "🌐 JSON-RPC request: {}", value.method);
    debug!(target: LOG_TARGET, "🌐 JSON-RPC body: {:?}", value);
    match value.method.as_str() {
        "rpc.discover" => handlers.rpc_discover(value),
        "get_identity" => handlers.get_identity(value).await,
        "get_all_vns" => handlers.get_all_vns(value).await,
        "add_peer" => handlers.add_peer(value).await,
        "get_comms_stats" => handlers.get_comms_stats(value).await,
        "get_substate" => handlers.get_substate(value).await,
        "inspect_substate" => handlers.inspect_substate(value).await,
        "get_addresses" => handlers.get_addresses(value).await,
        "add_address" => handlers.add_address(value).await,
        "delete_address" => handlers.delete_address(value).await,
        "clear_addresses" => handlers.clear_addresses(value).await,
        "get_connections" => handlers.get_connections(value).await,
        "get_non_fungible_collections" => handlers.get_non_fungible_collections(value).await,
        "get_non_fungible_count" => handlers.get_non_fungible_count(value).await,
        "get_non_fungibles" => handlers.get_non_fungibles(value).await,
        "submit_transaction" => handlers.submit_transaction(value).await,
        "get_transaction_result" => handlers.get_transaction_result(value).await,
        "get_substate_transactions" => handlers.get_substate_transactions(value).await,
        "get_epoch_manager_stats" => handlers.get_epoch_manager_stats(value).await,
        method => Ok(value.method_not_found(method)),
    }
}
