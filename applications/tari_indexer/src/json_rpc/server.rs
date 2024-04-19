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

use axum::{extract::Extension, middleware, routing::post, Router};
use axum_jrpc::{JrpcResult, JsonRpcExtractor};
use log::*;
use tower_http::cors::CorsLayer;

use super::handlers::JsonRpcHandlers;

const LOG_TARGET: &str = "tari::indexer::json_rpc";

pub fn spawn_json_rpc(preferred_address: SocketAddr, handlers: JsonRpcHandlers) -> anyhow::Result<SocketAddr> {
    let router = Router::new()
        .route("/", post(handler))
        .route("/json_rpc", post(handler))
        .layer(middleware::from_fn(logger::middleware_fn))
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
    let listen_addr = server.local_addr();
    info!(target: LOG_TARGET, "🌐 JSON-RPC listening on {listen_addr}");
    tokio::spawn(server);

    Ok(listen_addr)
}

async fn handler(Extension(handlers): Extension<Arc<JsonRpcHandlers>>, value: JsonRpcExtractor) -> JrpcResult {
    info!(target: LOG_TARGET, "🌐 JSON-RPC request: {}", value.method);
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
        "get_template_definition" => handlers.get_template_definition(value).await,
        "list_templates" => handlers.list_templates(value).await,
        method => Ok(value.method_not_found(method)),
    }
}

// TODO: this is a janky and fairly costly logger found on SO, replace with tracing middleware
mod logger {
    use async_graphql::futures_util;
    use axum::{
        body::{Body, Bytes, HttpBody},
        http::{Request, Response, StatusCode},
        middleware::Next,
        response::IntoResponse,
    };
    use libp2p::bytes::{Buf, BufMut};

    use super::*;

    // use Body as type here
    pub async fn middleware_fn(
        req: Request<Body>,
        next: Next<Body>,
    ) -> Result<impl IntoResponse, (StatusCode, String)> {
        let (parts, body) = req.into_parts();
        let req = Request::from_parts(parts, body);

        let res = next.run(req).await;

        let (parts, body) = res.into_parts();
        let body_bytes = to_bytes(body).await.unwrap();
        debug!(target: LOG_TARGET, "🌐 Response: {}", String::from_utf8_lossy(&body_bytes));

        Ok(Response::from_parts(parts, Body::from(body_bytes)))
    }

    async fn to_bytes<T>(body: T) -> Result<Bytes, T::Error>
    where T: HttpBody {
        futures_util::pin_mut!(body);

        // If there's only 1 chunk, we can just return Buf::to_bytes()
        let mut first = if let Some(buf) = body.data().await {
            buf?
        } else {
            return Ok(Bytes::new());
        };

        let second = if let Some(buf) = body.data().await {
            buf?
        } else {
            return Ok(first.copy_to_bytes(first.remaining()));
        };

        // Don't pre-emptively reserve *too* much.
        let rest = (body.size_hint().lower() as usize).min(1024 * 16);
        let cap = first
            .remaining()
            .saturating_add(second.remaining())
            .saturating_add(rest);
        // With more than 1 buf, we gotta flatten into a Vec first.
        let mut vec = Vec::with_capacity(cap);
        vec.put(first);
        vec.put(second);

        while let Some(buf) = body.data().await {
            vec.put(buf?);
        }

        Ok(vec.into())
    }
}
