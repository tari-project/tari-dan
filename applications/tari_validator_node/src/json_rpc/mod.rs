use axum::{extract::ContentLengthLimit, routing::post, Router};
use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use serde::{Deserialize, Serialize};
use tari_comms::{multiaddr::Multiaddr, peer_manager::NodeId, types::CommsPublicKey};
use tari_crypto::tari_utilities::hex::serialize_to_hex;

// curl 'http://127.0.0.1:13000/' -POST -d '{"jsonrpc": "2.0", "method": "div", "params": [7,0], "id": 1}' -H 'Content-Type: application/json'

pub async fn run_json_rpc() {
    let router = Router::new().route("/", post(handler));
    axum::Server::bind(&"127.0.0.1:13000".parse().unwrap())
        .serve(router.into_make_service())
        .await
        .unwrap();
}

async fn handler(ContentLengthLimit(value): ContentLengthLimit<JsonRpcExtractor, 1024>) -> JrpcResult {
    let answer_id = value.get_answer_id();
    println!("{:?}", value);
    match value.method.as_str() {
        "add" => {
            let request: Test = value.parse_params()?;
            let result = request.a + request.b;
            Ok(JsonRpcResponse::success(answer_id, result))
        },
        "sub" => {
            let result: [i32; 2] = value.parse_params()?;
            let result = match failing_sub(result[0], result[1]).await {
                Ok(result) => result,
                Err(e) => return Err(JsonRpcResponse::error(answer_id, e.into())),
            };
            Ok(JsonRpcResponse::success(answer_id, result))
        },
        "div" => {
            let result: [i32; 2] = value.parse_params()?;
            let result = match failing_div(result[0], result[1]).await {
                Ok(result) => result,
                Err(e) => return Err(JsonRpcResponse::error(answer_id, e.into())),
            };

            Ok(JsonRpcResponse::success(answer_id, result))
        },
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
        method => Ok(value.method_not_found(method)),
    }
}

async fn failing_sub(a: i32, b: i32) -> anyhow::Result<i32> {
    anyhow::ensure!(a > b, "a must be greater than b");
    Ok(a - b)
}

async fn failing_div(a: i32, b: i32) -> Result<i32, CustomError> {
    if b == 0 {
        Err(CustomError::DivideByZero)
    } else {
        Ok(a / b)
    }
}

#[derive(Deserialize, Debug)]
struct Test {
    a: i32,
    b: i32,
}

#[derive(Debug, thiserror::Error)]
enum CustomError {
    #[error("Divisor must not be equal to 0")]
    DivideByZero,
}

impl From<CustomError> for JsonRpcError {
    fn from(error: CustomError) -> Self {
        JsonRpcError::new(
            JsonRpcErrorReason::ServerError(-32099),
            error.to_string(),
            serde_json::Value::Null,
        )
    }
}

#[derive(Serialize, Debug)]
struct NodeIdentity {
    #[serde(serialize_with = "serialize_to_hex")]
    node_id: NodeId,
    public_key: CommsPublicKey,
    public_address: Multiaddr,
}
