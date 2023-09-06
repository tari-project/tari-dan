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
use indexmap::IndexMap;
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
use tari_crypto::tari_utilities::hex::Hex;
use tari_dan_app_utilities::template_manager::interface::TemplateManagerHandle;
use tari_dan_common_types::{optional::Optional, ShardId};
use tari_dan_storage::{
    consensus_models::{Block, ExecutedTransaction, QuorumDecision, SubstateRecord, TransactionRecord},
    Ordering,
    StateStore,
};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerReader};
use tari_state_store_sqlite::SqliteStateStore;
use tari_validator_node_client::types::{
    AddPeerRequest,
    AddPeerResponse,
    CommitteeShardInfo,
    DryRunTransactionFinalizeResult,
    GetCommitteeRequest,
    GetEpochManagerStatsResponse,
    GetIdentityResponse,
    GetNetworkCommitteeResponse,
    GetRecentTransactionsResponse,
    GetShardKey,
    GetStateRequest,
    GetStateResponse,
    GetSubstateRequest,
    GetSubstateResponse,
    GetSubstatesByTransactionRequest,
    GetSubstatesByTransactionResponse,
    GetTemplateRequest,
    GetTemplateResponse,
    GetTemplatesRequest,
    GetTemplatesResponse,
    GetTransactionRequest,
    GetTransactionResponse,
    GetTransactionResultRequest,
    GetTransactionResultResponse,
    GetValidatorFeesRequest,
    GetValidatorFeesResponse,
    SubmitTransactionRequest,
    SubmitTransactionResponse,
    SubstateStatus,
    TemplateMetadata,
    TemplateRegistrationRequest,
    TemplateRegistrationResponse,
};

use crate::{
    dry_run_transaction_processor::DryRunTransactionProcessor,
    grpc::base_layer_wallet::GrpcWalletClient,
    json_rpc::jrpc_errors::internal_error,
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
    base_node_client: GrpcBaseNodeClient,
    state_store: SqliteStateStore<PublicKey>,
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
            base_node_client,
            state_store: services.state_store.clone(),
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
        } = value.parse_params()?;
        debug!(
            target: LOG_TARGET,
            "Transaction {} has {} involved shards",
            transaction.hash(),
            transaction
                .num_involved_shards()
        );

        let tx_id = *transaction.id();

        if is_dry_run {
            let result = self
                .dry_run_transaction_processor
                .process_transaction(transaction)
                .await;
            match result {
                Ok(exec_result) => {
                    let response = SubmitTransactionResponse {
                        transaction_id: tx_id,
                        dry_run_result: Some(DryRunTransactionFinalizeResult {
                            decision: QuorumDecision::Accept,
                            finalize: exec_result.finalize,
                            transaction_failure: exec_result.transaction_failure,
                            fee_breakdown: exec_result.fee_receipt.map(|f| f.to_cost_breakdown()),
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

            Ok(JsonRpcResponse::success(answer_id, SubmitTransactionResponse {
                transaction_id: tx_id,
                dry_run_result: None,
            }))
        }
    }

    pub async fn get_state(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: GetStateRequest = value.parse_params()?;

        let mut tx = self.state_store.create_read_tx().unwrap();
        match SubstateRecord::get(&mut tx, &request.shard_id).optional() {
            Ok(Some(state)) => Ok(JsonRpcResponse::success(answer_id, GetStateResponse {
                data: state.into_substate().to_bytes(),
            })),
            Ok(None) => Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::ApplicationError(404),
                    "state not found".to_string(),
                    json::Value::Null,
                ),
            )),
            Err(e) => Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    format!("Something went wrong: {}", e),
                    json::Value::Null,
                ),
            )),
        }
    }

    pub async fn get_recent_transactions(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let mut tx = self.state_store.create_read_tx().unwrap();
        match TransactionRecord::get_paginated(&mut tx, 1000, 0, Some(Ordering::Descending)) {
            Ok(recent_transactions) => {
                let res = GetRecentTransactionsResponse {
                    transactions: recent_transactions.into_iter().map(|t| t.transaction).collect(),
                };
                Ok(JsonRpcResponse::success(answer_id, res))
            },
            Err(e) => Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    format!("Something went wrong: {}", e),
                    json::Value::Null,
                ),
            )),
        }
    }

    pub async fn get_transaction_result(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: GetTransactionResultRequest = value.parse_params()?;

        let mut tx = self.state_store.create_read_tx().map_err(internal_error(answer_id))?;
        let executed = ExecutedTransaction::get(&mut tx, &request.transaction_id)
            .optional()
            .map_err(internal_error(answer_id))?
            .ok_or_else(|| {
                JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(
                        JsonRpcErrorReason::ApplicationError(404),
                        format!("Transaction {} not found", request.transaction_id),
                        json::Value::Null,
                    ),
                )
            })?;

        let response = GetTransactionResultResponse {
            is_finalized: executed.is_finalized(),
            result: executed.into_final_result(),
        };
        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_transaction(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: GetTransactionRequest = value.parse_params()?;

        let transaction = self
            .state_store
            .with_read_tx(|tx| ExecutedTransaction::get(tx, &data.transaction_id))
            .map_err(internal_error(answer_id))?;

        Ok(JsonRpcResponse::success(answer_id, GetTransactionResponse {
            transaction,
        }))
    }

    pub async fn get_substate(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: GetSubstateRequest = value.parse_params()?;

        let maybe_substate = self
            .state_store
            .with_read_tx(|tx| {
                let shard_id = ShardId::from_address(&data.address, data.version);
                SubstateRecord::get(tx, &shard_id).optional()
            })
            .map_err(internal_error(answer_id))?;

        match maybe_substate {
            Some(substate) if substate.is_destroyed() => Ok(JsonRpcResponse::success(answer_id, GetSubstateResponse {
                status: SubstateStatus::Down,
                created_by_tx: Some(substate.created_by_transaction),
                value: None,
            })),
            Some(substate) => Ok(JsonRpcResponse::success(answer_id, GetSubstateResponse {
                status: SubstateStatus::Up,
                created_by_tx: Some(substate.created_by_transaction),
                value: Some(substate.into_substate_value()),
            })),
            None => Ok(JsonRpcResponse::success(answer_id, GetSubstateResponse {
                status: SubstateStatus::DoesNotExist,
                created_by_tx: None,
                value: None,
            })),
        }
    }

    pub async fn get_substates_created_by_transaction(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: GetSubstatesByTransactionRequest = value.parse_params()?;
        let substates = self
            .state_store
            .with_read_tx(|tx| SubstateRecord::get_many_by_created_transaction(tx, &data.transaction_id))
            .map_err(internal_error(answer_id))?;

        Ok(JsonRpcResponse::success(answer_id, GetSubstatesByTransactionResponse {
            substates,
        }))
    }

    pub async fn get_substates_destroyed_by_transaction(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: GetSubstatesByTransactionRequest = value.parse_params()?;
        let substates = self
            .state_store
            .with_read_tx(|tx| SubstateRecord::get_many_by_destroyed_transaction(tx, &data.transaction_id))
            .map_err(internal_error(answer_id))?;

        Ok(JsonRpcResponse::success(answer_id, GetSubstatesByTransactionResponse {
            substates,
        }))
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
                    JsonRpcErrorReason::InternalError,
                    format!("Could not get current epoch: {}", e),
                    json::Value::Null,
                ),
            )
        })?;
        let current_block_height = self.epoch_manager.current_block_height().await.map_err(|e| {
            JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InternalError,
                    format!("Could not get current block height: {}", e),
                    json::Value::Null,
                ),
            )
        })?;
        let committee_shard = self
            .epoch_manager
            .get_local_committee_shard(current_epoch)
            .await
            .map(Some)
            .or_else(|err| {
                if err.is_not_registered_error() {
                    Ok(None)
                } else {
                    Err(JsonRpcResponse::error(
                        answer_id,
                        JsonRpcError::new(
                            JsonRpcErrorReason::InternalError,
                            format!("Could not get committee shard:{}", err),
                            json::Value::Null,
                        ),
                    ))
                }
            })?;
        let response = GetEpochManagerStatsResponse {
            current_epoch,
            current_block_height,
            is_valid: committee_shard.is_some(),
            committee_shard,
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

    pub async fn get_network_committees(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let current_epoch = self
            .epoch_manager
            .current_epoch()
            .await
            .map_err(internal_error(answer_id))?;
        let num_committees = self
            .epoch_manager
            .get_num_committees(current_epoch)
            .await
            .map_err(internal_error(answer_id))?;

        let mut validators = self
            .epoch_manager
            .get_all_validator_nodes(current_epoch)
            .await
            .map_err(internal_error(answer_id))?;

        validators.sort_by(|vn_a, vn_b| vn_b.committee_bucket.cmp(&vn_a.committee_bucket));
        // Group by bucket, IndexMap used to preserve ordering
        let mut validators_per_bucket = IndexMap::with_capacity(validators.len());
        for validator in validators {
            validators_per_bucket
                .entry(
                    validator
                        .committee_bucket
                        .expect("validator committee bucket must have been populated within valid epoch"),
                )
                .or_insert_with(Vec::new)
                .push(validator);
        }

        let committees = validators_per_bucket
            .into_iter()
            .map(|(bucket, validators)| CommitteeShardInfo {
                bucket,
                shard_range: bucket.to_shard_range(num_committees),
                validators,
            })
            .collect();

        Ok(JsonRpcResponse::success(answer_id, GetNetworkCommitteeResponse {
            current_epoch,
            committees,
        }))
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

    pub async fn get_validator_fees(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request = value.parse_params::<GetValidatorFeesRequest>()?;

        let blocks = self
            .state_store
            .with_read_tx(|tx| {
                Block::get_any_with_epoch_range_for_validator(
                    tx,
                    request.epoch_range,
                    request.validator_public_key.as_ref(),
                )
            })
            .map_err(internal_error(answer_id))?;

        Ok(JsonRpcResponse::success(answer_id, GetValidatorFeesResponse {
            fees: blocks
                .into_iter()
                .filter(|b| b.total_leader_fee() > 0)
                .map(Into::into)
                .collect(),
        }))
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
