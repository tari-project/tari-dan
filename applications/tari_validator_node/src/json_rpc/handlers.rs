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

use std::{convert::TryInto, sync::Arc, time::Duration};

use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use log::*;
use serde::Serialize;
use serde_json::{self as json, json};
use tari_base_node_client::{grpc::GrpcBaseNodeClient, BaseNodeClient};
use tari_common_types::types::PublicKey;
use tari_comms::{
    multiaddr::Multiaddr,
    peer_manager::{NodeId, PeerFeatures},
    types::CommsPublicKey,
    CommsNode,
    NodeIdentity,
};
use tari_comms_logging::SqliteMessageLog;
use tari_crypto::tari_utilities::{hex::Hex, ByteArray};
use tari_dan_app_utilities::template_manager::TemplateManagerHandle;
use tari_dan_common_types::{optional::Optional, PayloadId, QuorumCertificate, QuorumDecision, ShardId};
use tari_dan_core::workers::events::{EventSubscription, HotStuffEvent};
use tari_dan_storage::{ShardStore, ShardStoreReadTransaction};
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStore;
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManager};
use tari_template_lib::Hash;
use tari_validator_node_client::types::{
    AddPeerRequest,
    AddPeerResponse,
    GetClaimableFeesRequest,
    GetClaimableFeesResponse,
    GetCommitteeRequest,
    GetEpochManagerStatsResponse,
    GetIdentityResponse,
    GetRecentTransactionsResponse,
    GetShardKey,
    GetStateRequest,
    GetStateResponse,
    GetSubstateRequest,
    GetSubstateResponse,
    GetTemplateRequest,
    GetTemplateResponse,
    GetTemplatesRequest,
    GetTemplatesResponse,
    GetTransactionQcsRequest,
    GetTransactionQcsResponse,
    GetTransactionResultRequest,
    GetTransactionResultResponse,
    SubmitTransactionRequest,
    SubmitTransactionResponse,
    SubstateStatus,
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
    grpc::base_layer_wallet::GrpcWalletClient,
    json_rpc::{
        jrpc_errors::{internal_error, invalid_argument},
        JsonTransactionResult,
    },
    p2p::services::mempool::MempoolHandle,
    registration,
    Services,
    ValidatorNodeConfig,
};

const LOG_TARGET: &str = "tari::validator_node::json_rpc::handlers";

pub struct JsonRpcHandlers {
    node_identity: Arc<NodeIdentity>,
    wallet_grpc_client: GrpcWalletClient,
    mempool: MempoolHandle,
    template_manager: TemplateManagerHandle,
    epoch_manager: EpochManagerHandle,
    comms: CommsNode,
    hotstuff_events: EventSubscription<HotStuffEvent<CommsPublicKey>>,
    base_node_client: GrpcBaseNodeClient,
    shard_store: SqliteShardStore,
    dry_run_transaction_processor: DryRunTransactionProcessor,
    config: ValidatorNodeConfig,
}

impl JsonRpcHandlers {
    pub fn new(
        wallet_grpc_client: GrpcWalletClient,
        base_node_client: GrpcBaseNodeClient,
        services: &Services,
        config: ValidatorNodeConfig,
    ) -> Self {
        Self {
            config,
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
            public_address: self.node_identity.public_addresses().first().unwrap().to_string(),
        };

        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn submit_transaction(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let SubmitTransactionRequest {
            transaction,
            is_dry_run,
            wait_for_result,
            wait_for_result_timeout,
        } = value.parse_params()?;
        debug!(
            target: LOG_TARGET,
            "Transaction {} has involved shards {:?}",
            transaction.hash(),
            transaction
                .meta()
                .involved_objects_iter()
                .map(|(s, ch)| format!("{}:{}", s, ch))
                .collect::<Vec<_>>()
        );

        // Pass to translation engine to translate into Shards and Substates.
        // TODO: submit the transaction to the wasm engine and return the result data
        let hash = *transaction.hash();

        if is_dry_run {
            let result = self
                .dry_run_transaction_processor
                .process_transaction(&transaction)
                .await;
            match result {
                Ok(exec_result) => {
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
                            finalize: exec_result.finalize,
                            transaction_failure: exec_result.transaction_failure,
                            fee_breakdown: exec_result.fee_receipt.map(|f| f.to_cost_breakdown()),
                            // TODO: Get correct QC
                            qc: QuorumCertificate::genesis(
                                epoch,
                                PayloadId::new(hash),
                                ShardId::zero(),
                                PublicKey::from_vec(&vec![0; 32]).unwrap(),
                            ),
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
            self.mempool.submit_transaction(transaction).await.map_err(|e| {
                log::error!(target: LOG_TARGET, "🚨 Mempool error: {}", e);
                JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(
                        JsonRpcErrorReason::InternalError,
                        format!("Mempool rejected transaction: {}", e),
                        serde_json::Value::Null,
                    ),
                )
            })?;

            if wait_for_result {
                return wait_for_transaction_result(
                    answer_id,
                    hash,
                    subscription,
                    Duration::from_secs(wait_for_result_timeout.unwrap_or(30)),
                )
                .await;
            }

            Ok(JsonRpcResponse::success(answer_id, SubmitTransactionResponse {
                hash: hash.into_array().into(),
                result: None,
            }))
        }
    }

    pub async fn get_state(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: GetStateRequest = value.parse_params()?;

        let mut tx = self.shard_store.create_read_tx().unwrap();
        let state = match tx.get_substate_states(&[request.shard_id]) {
            Ok(state) => state,
            Err(e) => {
                return Err(JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(
                        JsonRpcErrorReason::InvalidParams,
                        format!("Something went wrong: {}", e),
                        json::Value::Null,
                    ),
                ))
            },
        };
        if state.is_empty() {
            return Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::ApplicationError(404),
                    "state not found".to_string(),
                    json::Value::Null,
                ),
            ));
        }

        Ok(JsonRpcResponse::success(answer_id, GetStateResponse {
            data: state[0].substate().to_bytes(),
        }))
    }

    pub async fn get_recent_transactions(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let mut tx = self.shard_store.create_read_tx().unwrap();
        if let Ok(recent_transactions) = tx.get_recent_transactions() {
            let res = GetRecentTransactionsResponse {
                transactions: recent_transactions,
            };
            Ok(JsonRpcResponse::success(answer_id, res))
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
        let request: GetTransactionResultRequest = value.parse_params()?;
        let payload_id = PayloadId::new(request.hash);

        let mut tx = self.shard_store.create_read_tx().unwrap();
        let payload = tx
            .get_payload(&payload_id)
            .optional()
            .map_err(internal_error(answer_id))?
            .ok_or_else(|| {
                JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(
                        JsonRpcErrorReason::ApplicationError(404),
                        format!("Transaction with hash {} not found", payload_id),
                        json::Value::Null,
                    ),
                )
            })?;

        let response = GetTransactionResultResponse {
            result: payload.result().cloned(),
        };
        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_transaction_qcs(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: GetTransactionQcsRequest = value.parse_params()?;
        let payload_id = PayloadId::new(request.hash);

        let qcs = self
            .shard_store
            .with_read_tx(|tx| tx.get_high_qcs(payload_id))
            .map_err(internal_error(answer_id))?;

        let response = GetTransactionQcsResponse { qcs };
        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_transaction(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: TransactionRequest = value.parse_params()?;
        let mut tx = self.shard_store.create_read_tx().unwrap();
        let id = data
            .payload_id
            .clone()
            .try_into()
            .map_err(invalid_argument(answer_id))?;
        let dan_payload = tx.get_payload(&id).map_err(internal_error(answer_id))?;

        match tx.get_transaction(data.payload_id) {
            // TODO: return the transaction with the Response struct, and probably rename this jrpc method to
            // get_transaction_status
            Ok(transaction) => Ok(JsonRpcResponse::success(answer_id, JsonTransactionResult {
                nodes: transaction,
                payload: dan_payload,
            })),
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

    pub async fn get_substate(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: GetSubstateRequest = value.parse_params()?;
        let mut tx = self.shard_store.create_read_tx().unwrap();
        let shard_id = ShardId::from_address(&data.address, data.version);
        match tx.get_substate_states(&[shard_id]) {
            Ok(substates) => {
                let (value, tx_hash, status) = if substates.is_empty() {
                    (None, None, SubstateStatus::DoesNotExist)
                } else if substates[0].destroyed_height().is_some() {
                    (None, None, SubstateStatus::Down)
                } else {
                    (
                        Some(substates[0].substate().substate_value().clone()),
                        Some(substates[0].created_payload_id().into_array().into()),
                        SubstateStatus::Up,
                    )
                };
                Ok(JsonRpcResponse::success(answer_id, GetSubstateResponse {
                    status,
                    created_by_tx: tx_hash,
                    value,
                }))
            },
            Err(err) => {
                error!(target: LOG_TARGET, "[get_substate] error {}", err);
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
        let mut tx = self.shard_store.create_read_tx().unwrap();
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

    pub async fn get_fees(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: GetClaimableFeesRequest = value.parse_params()?;
        let mut tx = self.shard_store.create_read_tx().unwrap();
        match tx.get_fees_by_epoch(data.epoch, data.claim_leader_public_key.to_vec()) {
            Ok(claim_fees) => Ok(JsonRpcResponse::success(answer_id, GetClaimableFeesResponse {
                total_accrued_fees: claim_fees
                    .iter()
                    .map(|fees| {
                        if fees.destroyed_at_epoch.is_none() {
                            fees.fee_paid_for_created_justify
                        } else {
                            fees.fee_paid_for_destroyed_justify
                        }
                    })
                    .sum::<i64>() as u64,
            })),
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

    pub async fn get_current_leader_state(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: TransactionRequest = value.parse_params()?;
        let mut tx = self.shard_store.create_read_tx().unwrap();
        let payload_id = PayloadId::new(data.payload_id);
        match tx.get_current_leaders_states(&payload_id) {
            Ok(states) => Ok(JsonRpcResponse::success(answer_id, json!(states))),
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
        let size = self.mempool.get_mempool_size().await.map_err(|err| {
            error!(target: LOG_TARGET, "Error getting mempool size: {}", err);
            JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    "Something went wrong".to_string(),
                    json::Value::Null,
                ),
            )
        })?;
        let response = json!({ "size": size });
        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_epoch_manager_stats(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let current_epoch = self.epoch_manager.current_epoch().await.map_err(|e| {
            JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    format!("Could not get current epoch: {}", e),
                    json::Value::Null,
                ),
            )
        })?;
        let current_block_height = self.epoch_manager.current_block_height().await.map_err(|e| {
            JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    format!("Could not get current block height: {}", e),
                    json::Value::Null,
                ),
            )
        })?;

        let is_valid = self.epoch_manager.is_epoch_valid(current_epoch).await.map_err(|err| {
            JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    format!("Epoch is not valid:{}", err),
                    json::Value::Null,
                ),
            )
        })?;
        let response = GetEpochManagerStatsResponse {
            current_epoch,
            current_block_height,
            is_valid,
        };
        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn add_peer(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let AddPeerRequest {
            public_key,
            addresses,
            wait_for_dial,
        } = value.parse_params()?;

        let connectivity = self.comms.connectivity();
        let peer_manager = self.comms.peer_manager();

        let node_id = NodeId::from_public_key(&public_key);

        peer_manager
            .add_or_update_online_peer(
                &public_key,
                node_id.clone(),
                addresses,
                PeerFeatures::COMMUNICATION_NODE,
                &tari_comms::net_address::PeerAddressSource::Config,
            )
            .await
            .map_err(internal_error(answer_id))?;
        if wait_for_dial {
            let _conn = connectivity
                .dial_peer(node_id)
                .await
                .map_err(internal_error(answer_id))?;
        } else {
            // Dial without waiting
            connectivity
                .request_many_dials(Some(node_id))
                .await
                .map_err(internal_error(answer_id))?;
        }

        Ok(JsonRpcResponse::success(answer_id, AddPeerResponse {}))
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

    pub async fn get_logged_messages(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request = value.parse_params::<json::Value>()?;
        let logger = SqliteMessageLog::new(&self.config.p2p.datastore_path);
        let message_tag = request["message_tag"]
            .as_str()
            .map(ToString::to_string)
            .ok_or_else(|| {
                JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(
                        JsonRpcErrorReason::InvalidParams,
                        "message_tag is required".to_string(),
                        json::Value::Null,
                    ),
                )
            })?;
        let messages = logger.get_messages_by_tag(message_tag);
        let response = json!({ "messages": messages });
        Ok(JsonRpcResponse::success(answer_id, response))
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
    mut subscription: broadcast::Receiver<HotStuffEvent<CommsPublicKey>>,
    timeout: Duration,
) -> JrpcResult {
    loop {
        match tokio::time::timeout(timeout, subscription.recv()).await {
            Ok(res) => match res {
                Ok(HotStuffEvent::OnFinalized(qc, result)) => {
                    if qc.payload_id().as_bytes() != hash.as_ref() {
                        continue;
                    }

                    let response = SubmitTransactionResponse {
                        hash: hash.into_array().into(),
                        result: Some(TransactionFinalizeResult {
                            decision: *qc.decision(),
                            finalize: result.finalize,
                            transaction_failure: result.transaction_failure,
                            fee_breakdown: result.fee_receipt.map(|f| f.to_cost_breakdown()),
                            qc: *qc,
                        }),
                    };

                    return Ok(JsonRpcResponse::success(answer_id, response));
                },
                Ok(HotStuffEvent::Failed(payload_id, err)) => {
                    if payload_id.as_bytes() != hash.as_ref() {
                        continue;
                    }
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
