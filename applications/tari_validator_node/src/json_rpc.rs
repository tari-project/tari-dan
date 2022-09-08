//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ContentLengthLimit, Extension},
    routing::post,
    Router,
};
use axum_jrpc::{JrpcResult, JsonRpcExtractor, JsonRpcResponse};
use log::*;
use serde::Serialize;
use tari_comms::{multiaddr::Multiaddr, peer_manager::NodeId, types::CommsPublicKey, NodeIdentity};
use tari_crypto::tari_utilities::hex::serialize_to_hex;
use tari_dan_engine::instruction::Transaction;

const LOG_TARGET: &str = "tari::validator_node::json_rpc";
const JSON_SIZE_LIMIT_BYTES: u64 = 25 * 1024; // 25 kb

struct State {
    node_identity: NodeIdentity,
}

pub async fn run_json_rpc(address: SocketAddr, node_identity: NodeIdentity) -> Result<(), anyhow::Error> {
    let shared_state = Arc::new(State { node_identity });

    let router = Router::new().route("/", post(handler)).layer(Extension(shared_state));

    axum::Server::bind(&address)
        .serve(router.into_make_service())
        .await
        .map_err(|err| {
            error!(target: LOG_TARGET, "JSON-RPC encountered an error: {}", err);
            err
        })?;

    info!("Stopping JSON-RPC");
    info!(target: LOG_TARGET, "Stopping JSON-RPC");
    Ok(())
}

async fn handler(
    Extension(state): Extension<Arc<State>>,
    ContentLengthLimit(value): ContentLengthLimit<JsonRpcExtractor, JSON_SIZE_LIMIT_BYTES>,
) -> JrpcResult {
    let answer_id = value.get_answer_id();
    match value.method.as_str() {
        "get_identity" => {
            let response = NodeIdentityResponse {
                node_id: state.node_identity.node_id().clone(),
                public_key: state.node_identity.public_key().clone(),
                public_address: state.node_identity.public_address(),
            };

            Ok(JsonRpcResponse::success(answer_id, response))
        },
        "submit_transaction" => {
            let transaction: Transaction = value.parse_params()?;

            // TODO: submit the transaction to the wasm engine and return the result data
            println!("Transaction: {:?}", transaction);

            Ok(JsonRpcResponse::success(answer_id, ()))
        },
        method => Ok(value.method_not_found(method)),
    }
}

#[derive(Serialize, Debug)]
struct NodeIdentityResponse {
    #[serde(serialize_with = "serialize_to_hex")]
    node_id: NodeId,
    public_key: CommsPublicKey,
    public_address: Multiaddr,
}
