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

use std::{sync::Arc, time::Duration};

use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use log::*;
use serde::Serialize;
use serde_json::{self as json, json};
use tari_comms::{multiaddr::Multiaddr, peer_manager::NodeId, types::CommsPublicKey, CommsNode, NodeIdentity};
use tari_crypto::tari_utilities::hex::Hex;
use tari_dan_common_types::{PayloadId, QuorumCertificate, QuorumDecision, SubstateChange};
use tari_dan_core::{
    services::{epoch_manager::EpochManager, BaseNodeClient},
    storage::shard_store::{ShardStore, ShardStoreTransaction},
    workers::events::{EventSubscription, HotStuffEvent},
};
use tari_dan_engine::transaction::TransactionBuilder;
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStore;
use tari_template_lib::Hash;
use tari_validator_node_client::types::{
    GetCommitteeRequest,
    GetIdentityResponse,
    GetShardKey,
    GetTemplateRequest,
    GetTemplateResponse,
    GetTemplatesRequest,
    GetTemplatesResponse,
    GetTransactionRequest,
    GetTransactionResponse,
    SubmitTransactionRequest,
    SubmitTransactionResponse,
    SubstatesRequest,
    TemplateMetadata,
    TemplateRegistrationRequest,
    TemplateRegistrationResponse,
    TransactionFinalizeResult,
    TransactionRequest,
};
use tokio::sync::{broadcast, broadcast::error::RecvError};

use crate::{
    dry_run_transaction_processor::DryRunTransactionProcessor,
    grpc::services::{base_node_client::GrpcBaseNodeClient, wallet_client::GrpcWalletClient},
    json_rpc::jrpc_errors::internal_error,
    p2p::services::{
        epoch_manager::handle::EpochManagerHandle,
        mempool::MempoolHandle,
        template_manager::TemplateManagerHandle,
    },
    registration,
    Services,
};

const LOG_TARGET: &str = "tari::validator_node::json_rpc::handlers";

pub struct JsonRpcHandlers {
    node_identity: Arc<NodeIdentity>,
    wallet_grpc_client: GrpcWalletClient,
    mempool: MempoolHandle,
    template_manager: TemplateManagerHandle,
    epoch_manager: EpochManagerHandle,
    comms: CommsNode,
    hotstuff_events: EventSubscription<HotStuffEvent>,
    base_node_client: GrpcBaseNodeClient,
    shard_store: SqliteShardStore,
    dry_run_transaction_processor: DryRunTransactionProcessor,
}

impl JsonRpcHandlers {
    pub fn new(
        wallet_grpc_client: GrpcWalletClient,
        base_node_client: GrpcBaseNodeClient,
        services: &Services,
    ) -> Self {
        Self {
            node_identity: services.comms.node_identity(),
            wallet_grpc_client,
            mempool: services.mempool.clone(),
            epoch_manager: services.epoch_manager.clone(),
            template_manager: services.template_manager.clone(),
            comms: services.comms.clone(),
            hotstuff_events: services.hotstuff_events.clone(),
            base_node_client,
            shard_store: services.shard_store.clone(),
            dry_run_transaction_processor: services.dry_run_transaction_processor.clone(),
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
            node_id: self.node_identity.node_id().to_hex(),
            public_key: self.node_identity.public_key().to_hex(),
            public_address: self.node_identity.public_address().to_string(),
        };

        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn submit_transaction(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: SubmitTransactionRequest = value.parse_params()?;

        let mut builder = TransactionBuilder::new();
        builder
            .with_inputs(
                request
                    .inputs
                    .iter()
                    .filter_map(|(shard, change)| {
                        if *change == SubstateChange::Destroy {
                            Some(*shard)
                        } else {
                            None
                        }
                    })
                    .collect(),
            )
            .with_outputs(
                request
                    .inputs
                    .iter()
                    .filter_map(|(shard, change)| {
                        if *change == SubstateChange::Create {
                            Some(*shard)
                        } else {
                            None
                        }
                    })
                    .collect(),
            )
            .with_instructions(request.instructions)
            .with_new_outputs(request.num_outputs)
            .with_signature(request.signature)
            .with_sender_public_key(request.sender_public_key);

        let transaction = builder.build();
        info!(
            target: LOG_TARGET,
            "Transaction {} has involved shards {:?}",
            transaction.hash(),
            transaction
                .meta()
                .involved_objects_iter()
                .map(|(s, (ch, _))| format!("{}:{}", s, ch))
                .collect::<Vec<_>>()
        );

        // Pass to translation engine to translate into Shards and Substates.
        // TODO: submit the transaction to the wasm engine and return the result data
        let hash = *transaction.hash();

        if request.is_dry_run {
            let result = self
                .dry_run_transaction_processor
                .process_transaction(transaction)
                .await;
            match result {
                Ok(finalize_result) => {
                    let epoch = match self.epoch_manager.current_epoch().await {
                        Ok(epoch) => epoch,
                        Err(e) => {
                            return Err(JsonRpcResponse::error(
                                answer_id,
                                JsonRpcError::new(JsonRpcErrorReason::ApplicationError(1), e.to_string(), json!(null)),
                            ))
                        },
                    };

                    let response = SubmitTransactionResponse {
                        hash: hash.into_array().into(),
                        result: Some(TransactionFinalizeResult {
                            decision: QuorumDecision::Accept,
                            finalize: finalize_result,
                            qc: QuorumCertificate::genesis(epoch),
                        }),
                    };

                    Ok(JsonRpcResponse::success(answer_id, response))
                },
                Err(e) => Err(JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(JsonRpcErrorReason::ApplicationError(1), e.to_string(), json!(null)),
                )),
            }
        } else {
            let subscription = self.hotstuff_events.subscribe();
            // Submit to mempool.
            self.mempool
                .submit_transaction(transaction)
                .await
                .map_err(internal_error(answer_id))?;

            if request.wait_for_result {
                return wait_for_transaction_result(
                    answer_id,
                    hash,
                    subscription,
                    Duration::from_secs(request.wait_for_result_timeout.unwrap_or(30)),
                )
                .await;
            }

            Ok(JsonRpcResponse::success(answer_id, SubmitTransactionResponse {
                hash: hash.into_array().into(),
                result: None,
            }))
        }
    }

    pub async fn get_recent_transactions(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let tx = self.shard_store.create_tx().unwrap();
        if let Ok(recent_transactions) = tx.get_recent_transactions() {
            Ok(JsonRpcResponse::success(answer_id, json!(recent_transactions)))
        } else {
            Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    "Something went wrong".to_string(),
                    json::Value::Null,
                ),
            ))
        }
    }

    pub async fn get_transaction_result(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: GetTransactionRequest = value.parse_params()?;
        let payload_id = PayloadId::new(request.hash);

        let tx = self.shard_store.create_tx().unwrap();
        if let Ok(payload) = tx.get_payload(&payload_id) {
            let response = GetTransactionResponse {
                result: payload.result().clone(),
            };
            Ok(JsonRpcResponse::success(answer_id, response))
        } else {
            Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    "Something went wrong".to_string(),
                    json::Value::Null,
                ),
            ))
        }
    }

    pub async fn get_transaction(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: TransactionRequest = value.parse_params()?;
        let tx = self.shard_store.create_tx().unwrap();
        match tx.get_transaction(data.payload_id) {
            Ok(transaction) => Ok(JsonRpcResponse::success(answer_id, json!(transaction))),
            Err(err) => {
                println!("error {:?}", err);
                Err(JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(
                        JsonRpcErrorReason::InvalidParams,
                        "Something went wrong".to_string(),
                        json::Value::Null,
                    ),
                ))
            },
        }
    }

    pub async fn get_substates(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: SubstatesRequest = value.parse_params()?;
        let tx = self.shard_store.create_tx().unwrap();
        match tx.get_substates_for_payload(data.payload_id, data.shard_id) {
            Ok(substates) => Ok(JsonRpcResponse::success(answer_id, json!(substates))),
            Err(err) => {
                println!("error {:?}", err);
                Err(JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(
                        JsonRpcErrorReason::InvalidParams,
                        "Something went wrong".to_string(),
                        json::Value::Null,
                    ),
                ))
            },
        }
    }

    pub async fn register_validator_node(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();

        let resp = registration::register(self.wallet_client(), &self.node_identity, &self.epoch_manager)
            .await
            .map_err(internal_error(answer_id))?;

        if !resp.is_success {
            return Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::ApplicationError(1),
                    format!("Failed to register validator node: {}", resp.failure_message),
                    json::Value::Null,
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

        let resp = self
            .wallet_client()
            .register_template(&self.node_identity, data)
            .await
            .map_err(internal_error(answer_id))?;

        Ok(JsonRpcResponse::success(answer_id, TemplateRegistrationResponse {
            template_address: resp.template_address,
            transaction_id: resp.tx_id,
        }))
    }

    pub async fn get_templates(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let req: GetTemplatesRequest = value.parse_params()?;

        let templates = self
            .template_manager
            .get_templates(req.limit as usize)
            .await
            .map_err(internal_error(answer_id))?;

        Ok(JsonRpcResponse::success(answer_id, GetTemplatesResponse {
            templates: templates
                .into_iter()
                .map(|t| TemplateMetadata {
                    name: t.name,
                    address: t.address,
                    url: t.url,
                    binary_sha: t.binary_sha,
                    height: t.height,
                })
                .collect(),
        }))
    }

    pub async fn get_template(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let req: GetTemplateRequest = value.parse_params()?;

        let template = self
            .template_manager
            .get_template(req.template_address)
            .await
            .map_err(internal_error(answer_id))?;

        let abi = self
            .template_manager
            .load_template_abi(req.template_address)
            .await
            .map_err(internal_error(answer_id))?;

        Ok(JsonRpcResponse::success(answer_id, GetTemplateResponse {
            registration_metadata: TemplateMetadata {
                name: template.metadata.name,
                address: template.metadata.address,
                url: template.metadata.url,
                binary_sha: template.metadata.binary_sha,
                height: template.metadata.height,
            },
            abi,
        }))
    }

    pub async fn get_connections(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        if let Ok(active_connections) = self.comms.connectivity().get_active_connections().await {
            let mut response = GetConnectionsResponse { connections: vec![] };
            let peer_manager = self.comms.peer_manager();
            for conn in active_connections {
                let peer = peer_manager
                    .find_by_node_id(conn.peer_node_id())
                    .await
                    .expect("Unexpected peer database error")
                    .expect("Peer not found");

                //  response.connections.push(Connection { node_id: (), public_key: (), address: (), direction: (), age:
                //                                                                                      (), user_agent:
                // (), Info: () })
                response.connections.push(Connection {
                    node_id: peer.node_id,
                    public_key: peer.public_key,
                    address: conn.address().clone(),
                    direction: conn.direction().is_inbound(),
                    age: conn.age().as_secs(),
                });
            }
            Ok(JsonRpcResponse::success(answer_id, response))
        } else {
            Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    "Something went wrong".to_string(),
                    json::Value::Null,
                ),
            ))
        }
    }

    pub async fn get_mempool_stats(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let response = json!({"size": self.mempool.get_mempool_size()});
        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_epoch_manager_stats(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        if let Ok(current_epoch) = self.epoch_manager.current_epoch().await {
            if let Ok(is_valid) = self.epoch_manager.is_epoch_valid(current_epoch).await {
                let response = json!({ "current_epoch": current_epoch.0,"is_valid":is_valid });
                Ok(JsonRpcResponse::success(answer_id, response))
            } else {
                Err(JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(
                        JsonRpcErrorReason::InvalidParams,
                        "Something went wrong".to_string(),
                        json::Value::Null,
                    ),
                ))
            }
        } else {
            Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    "Something went wrong".to_string(),
                    json::Value::Null,
                ),
            ))
        }
    }

    pub async fn get_comms_stats(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        if let Ok(stats) = self.comms.connectivity().get_connectivity_status().await {
            let response = json!({ "connection_status": format!("{:?}", stats) });
            Ok(JsonRpcResponse::success(answer_id, response))
        } else {
            Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    "Something went wrong".to_string(),
                    json::Value::Null,
                ),
            ))
        }
    }

    pub async fn get_shard_key(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request = value.parse_params::<GetShardKey>()?;
        if let Ok(shard_key) = self
            .base_node_client()
            .get_shard_key(request.height, &request.public_key)
            .await
        {
            let response = json!({ "shard_key": shard_key });
            Ok(JsonRpcResponse::success(answer_id, response))
        } else {
            Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    "Something went wrong".to_string(),
                    json::Value::Null,
                ),
            ))
        }
    }

    pub async fn get_committee(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request = value.parse_params::<GetCommitteeRequest>()?;
        if let Ok(committee) = self.epoch_manager.get_committee(request.epoch, request.shard_id).await {
            let response = json!({ "committee": committee });
            Ok(JsonRpcResponse::success(answer_id, response))
        } else {
            Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    "Something went wrong".to_string(),
                    json::Value::Null,
                ),
            ))
        }
    }

    pub async fn get_all_vns(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let epoch: u64 = value.parse_params()?;
        if let Ok(vns) = self.base_node_client().get_validator_nodes(epoch * 10).await {
            let response = json!({ "vns": vns });
            Ok(JsonRpcResponse::success(answer_id, response))
        } else {
            Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    "Something went wrong".to_string(),
                    json::Value::Null,
                ),
            ))
        }
    }
}

#[derive(Serialize, Debug)]
struct Connection {
    node_id: NodeId,
    public_key: CommsPublicKey,
    address: Multiaddr,
    direction: bool,
    age: u64,
}

#[derive(Serialize, Debug)]
struct GetConnectionsResponse {
    connections: Vec<Connection>,
}

async fn wait_for_transaction_result(
    answer_id: i64,
    hash: Hash,
    mut subscription: broadcast::Receiver<HotStuffEvent>,
    timeout: Duration,
) -> JrpcResult {
    loop {
        match tokio::time::timeout(timeout, subscription.recv()).await {
            Ok(res) => match res {
                Ok(HotStuffEvent::OnFinalized(qc, result)) => {
                    if qc.payload_id().as_slice() != hash.as_ref() {
                        continue;
                    }

                    let response = SubmitTransactionResponse {
                        hash: hash.into_array().into(),
                        result: Some(TransactionFinalizeResult {
                            decision: *qc.decision(),
                            finalize: result,
                            qc: *qc,
                        }),
                    };

                    return Ok(JsonRpcResponse::success(answer_id, response));
                },
                Ok(HotStuffEvent::Failed(err)) => {
                    // May not be our tx that failed
                    warn!(target: LOG_TARGET, "Transaction failed: {}", err);
                    return Err(JsonRpcResponse::error(
                        answer_id,
                        JsonRpcError::new(
                            // TODO: define error code
                            JsonRpcErrorReason::ApplicationError(1),
                            err,
                            json!(null),
                        ),
                    ));
                },
                Err(RecvError::Lagged(n)) => {
                    error!(target: LOG_TARGET, "HotStuffEvent subscription lagged ({})", n);
                },
                Err(RecvError::Closed) => {
                    return Err(JsonRpcResponse::error(
                        answer_id,
                        JsonRpcError::new(
                            // TODO: define error code
                            JsonRpcErrorReason::ApplicationError(1),
                            "Failed to receive event".to_string(),
                            json!(null),
                        ),
                    ));
                },
            },
            Err(_) => {
                return Err(JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(
                        JsonRpcErrorReason::ApplicationError(2),
                        "Timeout waiting for result".to_string(),
                        json!(null),
                    ),
                ));
            },
        }
    }
}
