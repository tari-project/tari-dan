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

use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use indexmap::IndexMap;
use libp2p::swarm::dial_opts::{DialOpts, PeerCondition};
use log::*;
use serde_json::{self as json, json};
use tari_base_node_client::{grpc::GrpcBaseNodeClient, BaseNodeClient};
use tari_dan_app_utilities::{keypair::RistrettoKeypair, template_manager::interface::TemplateManagerHandle};
use tari_dan_common_types::{optional::Optional, public_key_to_peer_id, PeerAddress, ShardId};
use tari_dan_p2p::TariMessagingSpec;
use tari_dan_storage::{
    consensus_models::{Block, ExecutedTransaction, LeafBlock, QuorumDecision, SubstateRecord, TransactionRecord},
    Ordering,
    StateStore,
    StateStoreReadTransaction,
};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerReader};
use tari_networking::{is_supported_multiaddr, NetworkingHandle, NetworkingService};
use tari_state_store_sqlite::SqliteStateStore;
use tari_validator_node_client::{
    types,
    types::{
        AddPeerRequest,
        AddPeerResponse,
        CommitteeShardInfo,
        ConnectionDirection,
        DryRunTransactionFinalizeResult,
        GetBlockRequest,
        GetBlockResponse,
        GetBlocksCountResponse,
        GetCommitteeRequest,
        GetConnectionsResponse,
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
        ListBlocksRequest,
        ListBlocksResponse,
        RegisterValidatorNodeRequest,
        RegisterValidatorNodeResponse,
        SubmitTransactionRequest,
        SubmitTransactionResponse,
        SubstateStatus,
        TemplateMetadata,
        TemplateRegistrationRequest,
        TemplateRegistrationResponse,
    },
};

use crate::{
    dry_run_transaction_processor::DryRunTransactionProcessor,
    grpc::base_layer_wallet::GrpcWalletClient,
    json_rpc::jrpc_errors::{internal_error, not_found},
    p2p::services::mempool::MempoolHandle,
    registration,
    Services,
};

const LOG_TARGET: &str = "tari::validator_node::json_rpc::handlers";

pub struct JsonRpcHandlers {
    keypair: RistrettoKeypair,
    wallet_grpc_client: GrpcWalletClient,
    mempool: MempoolHandle,
    template_manager: TemplateManagerHandle,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    networking: NetworkingHandle<TariMessagingSpec>,
    base_node_client: GrpcBaseNodeClient,
    state_store: SqliteStateStore<PeerAddress>,
    dry_run_transaction_processor: DryRunTransactionProcessor,
}

impl JsonRpcHandlers {
    pub fn new(
        wallet_grpc_client: GrpcWalletClient,
        base_node_client: GrpcBaseNodeClient,
        services: &Services,
    ) -> Self {
        Self {
            keypair: services.keypair.clone(),
            wallet_grpc_client,
            mempool: services.mempool.clone(),
            epoch_manager: services.epoch_manager.clone(),
            template_manager: services.template_manager.clone(),
            networking: services.networking.clone(),
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
    pub async fn get_identity(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let info = self
            .networking
            .get_local_peer_info()
            .await
            .map_err(internal_error(answer_id))?;
        let response = GetIdentityResponse {
            peer_id: info.peer_id.to_string(),
            public_key: self.keypair.public_key().clone(),
            public_addresses: info.listen_addrs,
            supported_protocols: info.protocols.into_iter().map(|p| p.to_string()).collect(),
            protocol_version: info.protocol_version,
            user_agent: info.agent_version,
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
            transaction.num_involved_shards()
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

    pub async fn list_blocks(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let req = value.parse_params::<ListBlocksRequest>()?;
        let mut tx = self.state_store.create_read_tx().map_err(internal_error(answer_id))?;
        let start_block = match req.from_id {
            Some(id) => Block::get(&mut tx, &id)
                .optional()
                .map_err(internal_error(answer_id))?
                .ok_or_else(|| not_found(answer_id, format!("Block {} not found", id)))?,
            None => LeafBlock::get(&mut tx)
                .optional()
                .map_err(internal_error(answer_id))?
                .ok_or_else(|| not_found(answer_id, "No leaf block"))?
                .get_block(&mut tx)
                .map_err(internal_error(answer_id))?,
        };
        let blocks = start_block
            .get_parent_chain(&mut tx, req.limit)
            .map_err(internal_error(answer_id))?;

        let res = ListBlocksResponse { blocks };
        Ok(JsonRpcResponse::success(answer_id, res))
    }

    pub async fn get_tx_pool(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let tx_pool = self
            .state_store
            .with_read_tx(|tx| tx.transaction_pool_get_all())
            .map_err(internal_error(answer_id))?;
        let res = json!({ "tx_pool": tx_pool });
        Ok(JsonRpcResponse::success(answer_id, res))
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

    pub async fn get_block(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: GetBlockRequest = value.parse_params()?;
        let mut tx = self.state_store.create_read_tx().unwrap();
        match Block::get(&mut tx, &data.block_id) {
            Ok(block) => {
                let res = GetBlockResponse { block };
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

    pub async fn get_blocks_count(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let mut tx = self.state_store.create_read_tx().unwrap();
        match Block::get_count(&mut tx) {
            Ok(count) => {
                let res = GetBlocksCountResponse { count };
                Ok(JsonRpcResponse::success(answer_id, res))
            },
            Err(e) => Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InternalError,
                    format!("Something went wrong: {}", e),
                    json::Value::Null,
                ),
            )),
        }
    }

    pub async fn register_validator_node(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let req: RegisterValidatorNodeRequest = value.parse_params()?;

        // Ensure that the fee claim pk is set before registering
        self.epoch_manager
            .set_fee_claim_public_key(req.fee_claim_public_key)
            .await
            .map_err(internal_error(answer_id))?;

        let resp = registration::register(self.wallet_client(), &self.keypair, &self.epoch_manager)
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

        Ok(JsonRpcResponse::success(answer_id, RegisterValidatorNodeResponse {
            transaction_id: resp.transaction_id.into(),
        }))
    }

    pub async fn register_template(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: TemplateRegistrationRequest = value.parse_params()?;

        let resp = self
            .wallet_client()
            .register_template(&self.keypair, data)
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
        let active_connections = self
            .networking
            .get_active_connections()
            .await
            .map_err(internal_error(answer_id))?;

        let connections = active_connections
            .into_iter()
            .map(|conn| types::Connection {
                connection_id: conn.connection_id.to_string(),
                peer_id: conn.peer_id.to_string(),
                address: conn.endpoint.get_remote_address().clone(),
                direction: if conn.endpoint.is_dialer() {
                    ConnectionDirection::Outbound
                } else {
                    ConnectionDirection::Inbound
                },
                age: conn.age(),
                ping_latency: conn.ping_latency,
            })
            .collect();

        Ok(JsonRpcResponse::success(answer_id, GetConnectionsResponse {
            connections,
        }))
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

        if let Some(unsupported) = addresses.iter().find(|a| !is_supported_multiaddr(a)) {
            return Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    format!("Unsupported multiaddr {unsupported}"),
                    json::Value::Null,
                ),
            ));
        }

        let mut networking = self.networking.clone();
        let peer_id = public_key_to_peer_id(public_key);

        let dial_wait = networking
            .dial_peer(
                DialOpts::peer_id(peer_id)
                    .addresses(addresses)
                    .condition(PeerCondition::Always)
                    .build(),
            )
            .await
            .map_err(internal_error(answer_id))?;

        if wait_for_dial {
            dial_wait.await.map_err(internal_error(answer_id))?;
        }

        Ok(JsonRpcResponse::success(answer_id, AddPeerResponse {}))
    }

    pub async fn get_comms_stats(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let peers = self
            .networking
            .clone()
            .get_connected_peers()
            .await
            .map_err(internal_error(answer_id))?;

        let status = if peers.is_empty() { "Offline" } else { "Online" };
        let response = json!({ "connection_status": status });
        Ok(JsonRpcResponse::success(answer_id, response))
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

    pub async fn get_validator_fees(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request = value.parse_params::<GetValidatorFeesRequest>()?;

        let maybe_validator_addr = request.validator_public_key.as_ref();

        let blocks = self
            .state_store
            .with_read_tx(|tx| {
                Block::get_any_with_epoch_range_for_validator(tx, request.epoch_range, maybe_validator_addr)
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
