use axum::{extract::ContentLengthLimit, routing::post, Router};
use axum_jrpc::{JrpcResult, JsonRpcExtractor, JsonRpcResponse};
use serde::Serialize;
use tari_comms::{multiaddr::Multiaddr, peer_manager::NodeId, types::CommsPublicKey};
use tari_crypto::tari_utilities::hex::serialize_to_hex;
use tari_dan_engine::instruction::Transaction;

pub async fn run_json_rpc() {
    let router = Router::new().route("/", post(handler));
    axum::Server::bind(&"127.0.0.1:13000".parse().unwrap())
        .serve(router.into_make_service())
        .await
        .unwrap();
}

async fn handler(ContentLengthLimit(value): ContentLengthLimit<JsonRpcExtractor, 1024>) -> JrpcResult {
    let answer_id = value.get_answer_id();
    match value.method.as_str() {
        "get_identity" => {
            // TODO: retrieve the real identity
            let public_address: Multiaddr = "/ip4/127.0.0.1/udt/sctp/5678".parse().unwrap();
            let identity = NodeIdentity {
                node_id: NodeId::default(),
                public_key: CommsPublicKey::default(),
                public_address,
            };

            Ok(JsonRpcResponse::success(answer_id, identity))
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
struct NodeIdentity {
    #[serde(serialize_with = "serialize_to_hex")]
    node_id: NodeId,
    public_key: CommsPublicKey,
    public_address: Multiaddr,
}
