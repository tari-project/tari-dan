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

use std::sync::Arc;

use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use serde::Serialize;
use serde_json::json;
use tari_comms::{multiaddr::Multiaddr, peer_manager::NodeId, types::CommsPublicKey, NodeIdentity};
use tari_dan_engine::instruction::Transaction;

use crate::{grpc::services::wallet_client::GrpcWalletClient, json_rpc::jrpc_errors::internal_error};

const _LOG_TARGET: &str = "tari::validator_node::json_rpc::handlers";

pub struct JsonRpcHandlers {
    node_identity: Arc<NodeIdentity>,
    wallet_grpc_client: GrpcWalletClient,
}

impl JsonRpcHandlers {
    pub fn new(node_identity: Arc<NodeIdentity>, wallet_grpc_client: GrpcWalletClient) -> Self {
        Self {
            node_identity,
            wallet_grpc_client,
        }
    }

    pub fn wallet_client(&self) -> GrpcWalletClient {
        self.wallet_grpc_client.clone()
    }
}

impl JsonRpcHandlers {
    pub fn get_identity(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let response = GetIdentityResponse {
            node_id: self.node_identity.node_id().clone(),
            public_key: self.node_identity.public_key().clone(),
            public_address: self.node_identity.public_address(),
        };

        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn submit_transaction(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let transaction: Transaction = value.parse_params()?;

        // TODO: submit the transaction to the wasm engine and return the result data
        println!("Transaction: {:?}", transaction);

        Ok(JsonRpcResponse::success(answer_id, ()))
    }

    pub async fn register(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();

        let resp = self
            .wallet_client()
            .register_validator_node(&self.node_identity)
            .await
            .map_err(internal_error(answer_id))?;

        if !resp.is_success {
            return Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::ApplicationError(1),
                    format!("Failed to register validator node: {}", resp.failure_message),
                    serde_json::Value::Null,
                ),
            ));
        }

        Ok(JsonRpcResponse::success(
            answer_id,
            json!({ "transaction_id": resp.transaction_id }),
        ))
    }
}

#[derive(Serialize, Debug)]
struct GetIdentityResponse {
    node_id: NodeId,
    public_key: CommsPublicKey,
    public_address: Multiaddr,
}
