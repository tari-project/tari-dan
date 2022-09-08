use std::sync::Arc;

use axum::{
    extract::{ContentLengthLimit, Extension},
    routing::post,
    Router,
};
use axum_jrpc::{JrpcResult, JsonRpcExtractor, JsonRpcResponse};
use serde::Serialize;
use tari_comms::{multiaddr::Multiaddr, peer_manager::NodeId, types::CommsPublicKey, NodeIdentity};
use tari_crypto::tari_utilities::hex::serialize_to_hex;
use tari_dan_engine::instruction::Transaction;

struct State {
    node_identity: Arc<NodeIdentity>,
}

pub async fn run_json_rpc(node_identity: Arc<NodeIdentity>) {
    let shared_state = Arc::new(State { node_identity });

    let router = Router::new().route("/", post(handler)).layer(Extension(shared_state));

    axum::Server::bind(&"127.0.0.1:13000".parse().unwrap())
        .serve(router.into_make_service())
        .await
        .unwrap();
}

async fn handler(
    Extension(state): Extension<Arc<State>>,
    ContentLengthLimit(value): ContentLengthLimit<JsonRpcExtractor, 1024>,
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
            // TODO: submit the transaction to the wasm engine
            let transaction: Transaction = value.parse_params()?;
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
