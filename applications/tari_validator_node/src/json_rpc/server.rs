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
    extract::{ContentLengthLimit, Extension},
    routing::post,
    Router,
};
use axum_jrpc::{JrpcResult, JsonRpcExtractor};
use log::*;
use tower_http::cors::CorsLayer;

use super::handlers::JsonRpcHandlers;

const LOG_TARGET: &str = "tari::validator_node::json_rpc";
const JSON_SIZE_LIMIT_BYTES: u64 = 25 * 1024; // 25 kb

pub async fn run_json_rpc(address: SocketAddr, handlers: JsonRpcHandlers) -> Result<(), anyhow::Error> {
    let router = Router::new()
        .route("/", post(handler))
        .route("/json_rpc", post(handler))
        .layer(Extension(Arc::new(handlers)))
        .layer(CorsLayer::permissive());

    info!(target: LOG_TARGET, "🌐 RPC started at {}", address);
    axum::Server::bind(&address)
        .serve(router.into_make_service())
        .await
        .map_err(|err| {
            error!(target: LOG_TARGET, "JSON-RPC encountered an error: {}", err);
            err
        })?;

    info!(target: LOG_TARGET, "Stopping JSON-RPC");
    Ok(())
}

async fn handler(
    Extension(handlers): Extension<Arc<JsonRpcHandlers>>,
    ContentLengthLimit(value): ContentLengthLimit<JsonRpcExtractor, JSON_SIZE_LIMIT_BYTES>,
) -> JrpcResult {
    info!(target: LOG_TARGET, "🌐 JSON-RPC request: {}", value.method);
    match value.method.as_str() {
        "get_identity" => handlers.get_identity(value),
        "submit_transaction" => handlers.submit_transaction(value).await,
        "register_validator_node" => handlers.register_validator_node(value).await,
        "register_template" => handlers.register_template(value).await,
        "get_connections" => handlers.get_connections(value).await,
        "get_mempool_stats" => handlers.get_mempool_stats(value).await,
        "get_epoch_manager_stats" => handlers.get_epoch_manager_stats(value).await,
        "get_comms_stats" => handlers.get_comms_stats(value).await,
        "get_shard_key" => handlers.get_shard_key(value).await,
        "get_committee" => handlers.get_committee(value).await,
        "get_all_vns" => handlers.get_all_vns(value).await,
        method => Ok(value.method_not_found(method)),
    }
}
