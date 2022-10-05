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

use std::{convert::TryInto, sync::Arc};

use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use serde::Serialize;
use serde_json::json;
use tari_common_types::types::FixedHash;
use tari_comms::{multiaddr::Multiaddr, peer_manager::NodeId, types::CommsPublicKey, CommsNode, NodeIdentity};
use tari_dan_common_types::serde_with;
use tari_dan_core::services::{epoch_manager::EpochManager, BaseNodeClient};
use tari_dan_engine::transaction::{Instruction, TransactionBuilder};

use super::messages::GetCommitteeRequest;
use crate::{
    grpc::services::{
        base_node_client::GrpcBaseNodeClient,
        wallet_client::{GrpcWalletClient, TemplateRegistrationRequest},
    },
    json_rpc::{jrpc_errors::internal_error, messages::SubmitTransactionRequest},
    p2p::services::{epoch_manager::handle::EpochManagerHandle, mempool::MempoolHandle},
};

const _LOG_TARGET: &str = "tari::validator_node::json_rpc::handlers";

pub struct JsonRpcHandlers {
    node_identity: Arc<NodeIdentity>,
    wallet_grpc_client: GrpcWalletClient,
    mempool: MempoolHandle,
    epoch_manager: EpochManagerHandle,
    comms: CommsNode,
    base_node_client: GrpcBaseNodeClient,
}

impl JsonRpcHandlers {
    pub fn new(
        node_identity: Arc<NodeIdentity>,
        wallet_grpc_client: GrpcWalletClient,
        mempool: MempoolHandle,
        epoch_manager: EpochManagerHandle,
        comms: CommsNode,
        base_node_client: GrpcBaseNodeClient,
    ) -> Self {
        Self {
            node_identity,
            wallet_grpc_client,
            mempool,
            epoch_manager,
            comms,
            base_node_client,
        }
    }

    pub fn wallet_client(&self) -> GrpcWalletClient {
        self.wallet_grpc_client.clone()
    }

    pub fn base_node_client(&self) -> GrpcBaseNodeClient {
        self.base_node_client.clone()
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
        let transaction: SubmitTransactionRequest = value.parse_params()?;

        let mut builder = TransactionBuilder::new();
        builder.with_new_components(transaction.num_new_components);
        for i in transaction.instructions {
            builder.add_instruction(Instruction::CallFunction {
                template_address: i.template_address.into(),
                function: i.function,
                args: i.args.clone(),
            });
        }
        builder.signature(transaction.signature.try_into().map_err(internal_error(answer_id))?);
        builder.sender_public_key(transaction.sender_public_key);
        let mempool_tx = builder.build();

        // Pass to translation engine to translate into Shards and Substates.

        // Submit to mempool.

        // TODO: submit the transaction to the wasm engine and return the result data
        let hash = *mempool_tx.hash();
        self.mempool
            .new_transaction(mempool_tx)
            .await
            .map_err(internal_error(answer_id))?;

        Ok(JsonRpcResponse::success(answer_id, SubmitTransactionResponse {
            hash: hash.into_array().into(),
        }))
    }

    pub async fn register_validator_node(&self, value: JsonRpcExtractor) -> JrpcResult {
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

    pub async fn register_template(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: TemplateRegistrationRequest = value.parse_params()?;

        self.wallet_client()
            .register_template(&self.node_identity, data)
            .await
            .map_err(internal_error(answer_id))?;

        // TODO: add "transaction_id" to the grpc response
        Ok(JsonRpcResponse::success(answer_id, ()))
    }

    pub async fn get_connections(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let response = GetIdentityResponse {
            node_id: self.node_identity.node_id().clone(),
            public_key: self.node_identity.public_key().clone(),
            public_address: self.node_identity.public_address(),
        };

        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_mempool_stats(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let response = json!({"size": self.mempool.get_mempool_size()});
        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_epoch_manager_stats(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let current_epoch = self.epoch_manager.current_epoch().await.unwrap();
        let is_valid = self.epoch_manager.is_epoch_valid(current_epoch).await.unwrap();
        let response = json!({ "current_epoch": current_epoch.0,"is_valid":is_valid });
        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_comms_stats(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let stats = self.comms.connectivity().get_connectivity_status().await.unwrap();
        let response = json!({ "connection_status": format!("{:?}", stats) });
        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_shard_key(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let height: u64 = value.parse_params()?;
        let shard_key = self
            .base_node_client()
            .get_shard_key(height, self.node_identity.public_key())
            .await
            .unwrap();
        let response = json!({ "shard_key": shard_key });
        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_committee(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request = value.parse_params::<GetCommitteeRequest>()?;
        if let Ok(committee) = self.epoch_manager.get_committee(request.epoch, request.shard_id).await {
            println!("committee {:?}", committee);
            let response = json!({ "committee": committee });
            Ok(JsonRpcResponse::success(answer_id, response))
        } else {
            Err(JsonRpcResponse::error(
                1,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    "Something went wrong".to_string(),
                    serde_json::Value::Null,
                ),
            ))
        }
    }

    pub async fn get_all_vns(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let epoch: u64 = value.parse_params()?;
        let vns = self.base_node_client().get_validator_nodes(epoch * 10).await.unwrap();
        let response = json!({ "vns": vns });
        Ok(JsonRpcResponse::success(answer_id, response))
    }
}

#[derive(Serialize, Debug)]
struct GetIdentityResponse {
    #[serde(with = "serde_with::hex")]
    node_id: NodeId,
    #[serde(with = "serde_with::hex")]
    public_key: CommsPublicKey,
    public_address: Multiaddr,
}

#[derive(Serialize, Debug)]
struct SubmitTransactionResponse {
    #[serde(with = "serde_with::base64")]
    hash: FixedHash,
}
