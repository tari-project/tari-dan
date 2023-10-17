//   Copyright 2023. The Tari Project
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

use std::{collections::HashMap, fmt::Display, sync::Arc};

use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use log::{error, warn};
use serde::Serialize;
use serde_json::{self as json, json, Value};
use tari_base_node_client::{grpc::GrpcBaseNodeClient, types::BaseLayerConsensusConstants, BaseNodeClient};
use tari_comms::{
    multiaddr::Multiaddr,
    peer_manager::{NodeId, PeerFeatures},
    types::CommsPublicKey,
    CommsNode,
    NodeIdentity,
};
use tari_crypto::tari_utilities::hex::Hex;
use tari_dan_common_types::{optional::Optional, Epoch};
use tari_dan_storage::consensus_models::Decision;
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerReader};
use tari_indexer_client::types::{
    AddAddressRequest,
    AddPeerRequest,
    AddPeerResponse,
    DeleteAddressRequest,
    GetEpochManagerStatsResponse,
    GetIdentityResponse,
    GetNonFungibleCountRequest,
    GetNonFungiblesRequest,
    GetNonFungiblesResponse,
    GetRelatedTransactionsRequest,
    GetRelatedTransactionsResponse,
    GetSubstateRequest,
    GetSubstateResponse,
    GetTransactionResultRequest,
    GetTransactionResultResponse,
    IndexerTransactionFinalizedResult,
    InspectSubstateRequest,
    InspectSubstateResponse,
    NonFungibleSubstate,
    SubmitTransactionRequest,
    SubmitTransactionResponse,
};
use tari_validator_node_rpc::client::{SubstateResult, TariCommsValidatorNodeClientFactory, TransactionResultStatus};

use super::json_encoding::{encode_execute_result_into_json, encode_finalized_result_into_json};
use crate::{
    bootstrap::Services,
    dry_run::processor::DryRunTransactionProcessor,
    json_rpc::json_encoding::encode_substate_into_json,
    substate_manager::SubstateManager,
    transaction_manager::TransactionManager,
};

const LOG_TARGET: &str = "tari::indexer::json_rpc::handlers";

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

pub struct JsonRpcHandlers {
    consensus_constants: BaseLayerConsensusConstants,
    node_identity: Arc<NodeIdentity>,
    comms: CommsNode,
    base_node_client: GrpcBaseNodeClient,
    substate_manager: Arc<SubstateManager>,
    epoch_manager: EpochManagerHandle,
    transaction_manager: TransactionManager<EpochManagerHandle, TariCommsValidatorNodeClientFactory>,
    dry_run_transaction_processor: DryRunTransactionProcessor<EpochManagerHandle, TariCommsValidatorNodeClientFactory>,
}

impl JsonRpcHandlers {
    pub fn new(
        consensus_constants: BaseLayerConsensusConstants,
        services: &Services,
        base_node_client: GrpcBaseNodeClient,
        substate_manager: Arc<SubstateManager>,
        transaction_manager: TransactionManager<EpochManagerHandle, TariCommsValidatorNodeClientFactory>,
        dry_run_transaction_processor: DryRunTransactionProcessor<
            EpochManagerHandle,
            TariCommsValidatorNodeClientFactory,
        >,
    ) -> Self {
        Self {
            consensus_constants,
            node_identity: services.comms.node_identity(),
            comms: services.comms.clone(),
            base_node_client,
            substate_manager,
            epoch_manager: services.epoch_manager.clone(),
            transaction_manager,
            dry_run_transaction_processor,
        }
    }

    pub fn base_node_client(&self) -> GrpcBaseNodeClient {
        self.base_node_client.clone()
    }
}

impl JsonRpcHandlers {
    pub fn rpc_discover(&self, value: JsonRpcExtractor) -> JrpcResult {
        Ok(JsonRpcResponse::success(
            value.id,
            serde_json::from_str::<HashMap<String, Value>>(include_str!("../../openrpc.json")).map_err(|e| {
                JsonRpcResponse::error(
                    value.id,
                    JsonRpcError::new(JsonRpcErrorReason::InternalError, e.to_string(), json!({})),
                )
            })?,
        ))
    }

    pub fn get_identity(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let response = GetIdentityResponse {
            node_id: self.node_identity.node_id().to_hex(),
            public_key: self.node_identity.public_key().clone(),
            public_addresses: self.node_identity.public_addresses(),
        };

        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_all_vns(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let epoch: u64 = value.parse_params()?;
        let epoch_blocks = self.consensus_constants.epoch_to_height(Epoch(epoch));
        match self.base_node_client().get_validator_nodes(epoch_blocks).await {
            Ok(vns) => {
                let response = json!({ "vns": vns });
                Ok(JsonRpcResponse::success(answer_id, response))
            },
            Err(e) => Err(Self::internal_error(answer_id, format!("Failed to get all vns: {}", e))),
        }
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
            .map_err(|e| Self::internal_error(answer_id, format!("Could not update peer: {}", e)))?;
        if wait_for_dial {
            let _conn = connectivity
                .dial_peer(node_id)
                .await
                .map_err(|e| Self::internal_error(answer_id, e.to_string()))?;
        } else {
            // Dial without waiting
            connectivity
                .request_many_dials(Some(node_id))
                .await
                .map_err(|e| Self::internal_error(answer_id, e.to_string()))?;
        }

        Ok(JsonRpcResponse::success(answer_id, AddPeerResponse {}))
    }

    pub async fn get_comms_stats(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        match self.comms.connectivity().get_connectivity_status().await {
            Ok(stats) => {
                let response = json!({ "connection_status": format!("{:?}", stats) });
                Ok(JsonRpcResponse::success(answer_id, response))
            },
            Err(e) => {
                warn!(target: LOG_TARGET, "Failed to get comms stats: {}", e);
                Err(Self::internal_error(
                    answer_id,
                    format!("Failed to get comms stats: {}", e),
                ))
            },
        }
    }

    pub async fn get_connections(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        match self.comms.connectivity().get_active_connections().await {
            Ok(active_connections) => {
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
            },
            Err(e) => {
                warn!(target: LOG_TARGET, "Failed to get connections: {}", e);
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
        let request: GetSubstateRequest = value.parse_params()?;

        match self
            .substate_manager
            .get_substate(&request.address, request.version)
            .await
            .map_err(|e| {
                warn!(target: LOG_TARGET, "Error getting substate: {}", e);
                Self::internal_error(answer_id, format!("Error getting substate: {}", e))
            })? {
            Some(substate_resp) => Ok(JsonRpcResponse::success(answer_id, GetSubstateResponse {
                address: substate_resp.address,
                version: substate_resp.version,
                substate: substate_resp.substate,
                created_by_transaction: substate_resp.created_by_transaction,
            })),
            None => {
                if request.local_search_only {
                    Err(JsonRpcResponse::error(
                        answer_id,
                        JsonRpcError::new(
                            JsonRpcErrorReason::ApplicationError(404),
                            format!(
                                "Substate {} (version:>={}) not found",
                                request.address,
                                request.version.unwrap_or(0)
                            ),
                            Value::Null,
                        ),
                    ))
                } else {
                    // Ask network
                    let substate = self
                        .transaction_manager
                        .get_substate(request.address.clone(), request.version.unwrap_or_default())
                        .await
                        .map_err(|e| {
                            warn!(target: LOG_TARGET, "Error asking network for substate: {}", e);
                            JsonRpcResponse::error(
                                answer_id,
                                JsonRpcError::new(
                                    JsonRpcErrorReason::ApplicationError(501),
                                    format!("Error asking network for substate:{}", e),
                                    Value::Null,
                                ),
                            )
                        })?;
                    match substate {
                        SubstateResult::DoesNotExist => Err(JsonRpcResponse::error(
                            answer_id,
                            JsonRpcError::new(
                                JsonRpcErrorReason::ApplicationError(404),
                                format!(
                                    "Substate {} (version:>={}) not found, and not found on network",
                                    request.address,
                                    request.version.unwrap_or(0)
                                ),
                                Value::Null,
                            ),
                        )),
                        SubstateResult::Up {
                            address,
                            substate,
                            created_by_tx,
                        } => Ok(JsonRpcResponse::success(answer_id, GetSubstateResponse {
                            address,
                            version: substate.version(),
                            substate,
                            created_by_transaction: created_by_tx,
                        })),
                        SubstateResult::Down { version, .. } => Err(JsonRpcResponse::error(
                            answer_id,
                            JsonRpcError::new(
                                JsonRpcErrorReason::ApplicationError(301),
                                format!(
                                    "Substate {} (version:>={}) not found, but found in a down state on network at \
                                     version {}",
                                    request.address,
                                    request.version.unwrap_or(0) + 1,
                                    version
                                ),
                                Value::Null,
                            ),
                        )),
                    }
                }
            },
        }
    }

    pub async fn inspect_substate(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: InspectSubstateRequest = value.parse_params()?;

        let resp = self
            .substate_manager
            .get_substate(&request.address, request.version)
            .await
            .map_err(|e| {
                warn!(target: LOG_TARGET, "Error getting substate: {}", e);
                Self::internal_error(answer_id, format!("Error getting substate: {}", e))
            })?
            .ok_or_else(|| {
                JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(
                        JsonRpcErrorReason::ApplicationError(404),
                        format!(
                            "Substate {} (version:>={}) not found",
                            request.address,
                            request.version.unwrap_or(0)
                        ),
                        Value::Null,
                    ),
                )
            })?;

        Ok(JsonRpcResponse::success(answer_id, InspectSubstateResponse {
            address: resp.address,
            version: resp.version,
            substate_contents: encode_substate_into_json(&resp.substate)
                .map_err(|e| Self::internal_error(answer_id, e))?,
            created_by_transaction: resp.created_by_transaction,
        }))
    }

    pub async fn get_addresses(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();

        let res = self.substate_manager.get_all_addresses_from_db().await;

        match res {
            Ok(addresses) => Ok(JsonRpcResponse::success(answer_id, addresses)),
            Err(e) => {
                warn!(target: LOG_TARGET, "Error getting addresses: {}", e);
                Err(Self::internal_error(
                    answer_id,
                    format!("Error getting addresses: {}", e),
                ))
            },
        }
    }

    pub async fn add_address(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: AddAddressRequest = value.parse_params()?;

        match self
            .substate_manager
            .fetch_and_add_substate_to_db(&request.address)
            .await
        {
            Ok(_) => Ok(JsonRpcResponse::success(answer_id, ())),
            Err(e) => {
                warn!(target: LOG_TARGET, "Error adding address: {}", e);
                Err(Self::internal_error(answer_id, format!("Error adding address: {}", e)))
            },
        }
    }

    pub async fn delete_address(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: DeleteAddressRequest = value.parse_params()?;

        match self.substate_manager.delete_substate_from_db(&request.address).await {
            Ok(_) => Ok(JsonRpcResponse::success(answer_id, ())),
            Err(e) => {
                warn!(target: LOG_TARGET, "Error deleting address: {}", e);
                Err(Self::internal_error(
                    answer_id,
                    format!("Error deleting address: {}", e),
                ))
            },
        }
    }

    pub async fn clear_addresses(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();

        match self.substate_manager.delete_all_substates_from_db().await {
            Ok(_) => Ok(JsonRpcResponse::success(answer_id, ())),
            Err(e) => {
                warn!(target: LOG_TARGET, "Error clearing addresses: {}", e);
                Err(Self::internal_error(
                    answer_id,
                    format!("Error clearing addresses: {}", e),
                ))
            },
        }
    }

    pub async fn get_non_fungible_collections(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();

        let res = self.substate_manager.get_non_fungible_collections().await;

        match res {
            Ok(collections) => Ok(JsonRpcResponse::success(answer_id, collections)),
            Err(e) => {
                warn!(target: LOG_TARGET, "Error getting non fungible collections: {}", e);
                Err(Self::internal_error(
                    answer_id,
                    format!("Error getting non fungible collections: {}", e),
                ))
            },
        }
    }

    pub async fn get_non_fungible_count(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: GetNonFungibleCountRequest = value.parse_params()?;
        let count = self
            .substate_manager
            .get_non_fungible_count(&request.address)
            .await
            .map_err(|e| {
                warn!(target: LOG_TARGET, "Error getting non fungible count: {}", e);
                Self::internal_error(answer_id, format!("Error getting non fungible count: {}", e))
            })?;

        Ok(JsonRpcResponse::success(answer_id, count))
    }

    pub async fn get_non_fungibles(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: GetNonFungiblesRequest = value.parse_params()?;

        let res = self
            .substate_manager
            .get_non_fungibles(&request.address, request.start_index, request.end_index)
            .await
            .map_err(|e| Self::internal_error(answer_id, e))?;

        Ok(JsonRpcResponse::success(answer_id, GetNonFungiblesResponse {
            non_fungibles: res
                .into_iter()
                .map(|v| NonFungibleSubstate {
                    index: v.index,
                    address: v.address,
                    substate: v.substate,
                })
                .collect(),
        }))
    }

    pub async fn submit_transaction(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: SubmitTransactionRequest = value.parse_params()?;

        if request.is_dry_run {
            let transaction_id = *request.transaction.id();
            let exec_result = self
                .dry_run_transaction_processor
                .process_transaction(request.transaction, request.required_substates)
                .await
                .map_err(|e| Self::internal_error(answer_id, e))?;

            let json_results =
                encode_execute_result_into_json(&exec_result).map_err(|e| Self::internal_error(answer_id, e))?;

            Ok(JsonRpcResponse::success(answer_id, SubmitTransactionResponse {
                result: IndexerTransactionFinalizedResult::Finalized {
                    execution_result: Some(exec_result),
                    final_decision: Decision::Commit,
                    abort_details: None,
                    json_results,
                },
                transaction_id,
            }))
        } else {
            let transaction_id = self
                .transaction_manager
                .submit_transaction(request.transaction, request.required_substates)
                .await
                .map_err(|e| Self::internal_error(answer_id, e))?;

            Ok(JsonRpcResponse::success(answer_id, SubmitTransactionResponse {
                result: IndexerTransactionFinalizedResult::Pending,
                transaction_id,
            }))
        }
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

        let response = GetEpochManagerStatsResponse {
            current_epoch,
            current_block_height,
        };
        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_transaction_result(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: GetTransactionResultRequest = value.parse_params()?;

        let maybe_result = self
            .transaction_manager
            .get_transaction_result(request.transaction_id)
            .await
            .optional()
            .map_err(|e| Self::internal_error(answer_id, e))?;

        let result = maybe_result.ok_or_else(|| Self::not_found(answer_id, "Transaction not found"))?;

        let resp = match result {
            TransactionResultStatus::Pending => GetTransactionResultResponse {
                result: IndexerTransactionFinalizedResult::Pending,
            },
            TransactionResultStatus::Finalized(finalized) => {
                let json_results =
                    encode_finalized_result_into_json(&finalized).map_err(|e| Self::internal_error(answer_id, e))?;
                GetTransactionResultResponse {
                    result: IndexerTransactionFinalizedResult::Finalized {
                        final_decision: finalized.final_decision,
                        execution_result: finalized.execute_result,
                        abort_details: finalized.abort_details,
                        json_results,
                    },
                }
            },
        };

        Ok(JsonRpcResponse::success(answer_id, resp))
    }

    pub async fn get_substate_transactions(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: GetRelatedTransactionsRequest = value.parse_params()?;

        let mut version = request.version.unwrap_or(0);
        let mut transaction_ids = vec![];

        loop {
            let res = self
                .substate_manager
                .get_specific_substate(&request.address, version)
                .await;

            if let Ok(substate_result) = res {
                let transaction_id = match substate_result {
                    SubstateResult::DoesNotExist => break,
                    SubstateResult::Up { created_by_tx, .. } => created_by_tx,
                    SubstateResult::Down { deleted_by_tx, .. } => deleted_by_tx,
                };
                transaction_ids.push(transaction_id);
                version += 1;
            } else {
                break;
            }
        }

        // the last transaction may both down and up a substate
        transaction_ids.dedup();

        let mut transaction_results = vec![];
        for transaction_id in transaction_ids {
            let transaction_result = self
                .transaction_manager
                .get_transaction_result(transaction_id)
                .await
                .map_err(|e| Self::internal_error(answer_id, e))?;

            let indexer_transaction_result = match transaction_result {
                TransactionResultStatus::Pending => IndexerTransactionFinalizedResult::Pending,
                TransactionResultStatus::Finalized(finalized) => {
                    let json_results = encode_finalized_result_into_json(&finalized)
                        .map_err(|e| Self::internal_error(answer_id, e))?;
                    IndexerTransactionFinalizedResult::Finalized {
                        final_decision: finalized.final_decision,
                        execution_result: finalized.execute_result,
                        abort_details: finalized.abort_details,
                        json_results,
                    }
                },
            };

            transaction_results.push(indexer_transaction_result);
        }

        let resp = GetRelatedTransactionsResponse { transaction_results };

        Ok(JsonRpcResponse::success(answer_id, resp))
    }

    fn error_response<T: Display>(answer_id: i64, reason: JsonRpcErrorReason, message: T) -> JsonRpcResponse {
        JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(reason, message.to_string(), json::Value::Null),
        )
    }

    fn not_found<T: Display>(answer_id: i64, details: T) -> JsonRpcResponse {
        Self::error_response(answer_id, JsonRpcErrorReason::ApplicationError(404), details)
    }

    fn internal_error<T: Display>(answer_id: i64, error: T) -> JsonRpcResponse {
        let msg = if cfg!(debug_assertions) || option_env!("CI").is_some() {
            error.to_string()
        } else {
            error!(target: LOG_TARGET, "Internal error: {}", error);
            "Something went wrong".to_string()
        };
        Self::error_response(answer_id, JsonRpcErrorReason::InternalError, msg)
    }
}
