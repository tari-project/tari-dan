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
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
    error::{JsonRpcError, JsonRpcErrorReason},
};
use libp2p::swarm::dial_opts::{DialOpts, PeerCondition};
use log::*;
use ootle_byte_type::{ConvertFromByteType, ToByteType};
use serde_json::{self as json, json};
use tari_base_node_client::types::BaseLayerValidatorNode;
use tari_common_types::types::CompressedPublicKey;
use tari_consensus::{consensus_constants::ConsensusConstants, hotstuff::ConsensusCurrentState};
use tari_consensus_types::{Decision, LeafBlock};
use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
use tari_epoch_manager::{EpochManagerReader, service::EpochManagerHandle, traits::LayerOneTransactionSubmitter};
use tari_epoch_oracles::store::StoreKey;
use tari_networking::{NetworkingHandle, NetworkingService, is_supported_multiaddr};
use tari_ootle_app_utilities::keypair::RistrettoKeypair;
use tari_ootle_common_types::{
    Epoch,
    SubstateAddress,
    layer_one_transaction::{
        LayerOnePayloadType,
        LayerOneTransactionDef,
        ValidatorExitParams,
        ValidatorRegistrationParams,
    },
    optional::Optional,
    services::template_provider::TemplateProvider,
};
use tari_ootle_p2p::{PeerAddress, TariMessagingSpec, public_key_to_peer_id};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StorageError,
    consensus_models::{Block, BookkeepingModel, SubstateRecord, TransactionExecution, TransactionRecord},
    global::GlobalDb,
};
use tari_ootle_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_template_lib::types::crypto::{RistrettoPublicKeyBytes, Scalar32Bytes, SchnorrSignatureBytes};
use tari_transaction_components::transaction_components::ValidatorNodeSignature;
use tari_validator_node_client::types::{
    self,
    AddPeerRequest,
    AddPeerResponse,
    ConnectionDirection,
    FunctionDef,
    GetAllVnsRequest,
    GetAllVnsResponse,
    GetBlockRequest,
    GetBlockResponse,
    GetBlocksCountResponse,
    GetBlocksRequest,
    GetBlocksResponse,
    GetCommitteeRequest,
    GetCommitteeResponse,
    GetCommsStatsResponse,
    GetConnectionsResponse,
    GetConsensusStatusResponse,
    GetEpochManagerStatsResponse,
    GetFilteredBlocksCountRequest,
    GetIdentityResponse,
    GetMempoolStatsResponse,
    GetShardKeyRequest,
    GetShardKeyResponse,
    GetStateRequest,
    GetStateResponse,
    GetSubstateRequest,
    GetSubstateResponse,
    GetTemplateRequest,
    GetTemplateResponse,
    GetTransactionRequest,
    GetTransactionResponse,
    GetTransactionResultRequest,
    GetTransactionResultResponse,
    LayerOneTransactionParams,
    ListBlocksRequest,
    ListBlocksResponse,
    SubmitTransactionRequest,
    SubmitTransactionResponse,
    SubstateStatus,
    TemplateAbi,
    TemplateMetadata,
};

use crate::{
    ApplicationConfig,
    bootstrap::Services,
    consensus::{
        ConsensusHandle,
        spec::{ValidatorNodeStateStore, ValidatorTemplateProvider},
    },
    file_l1_submitter::FileLayerOneSubmitter,
    json_rpc::jrpc_errors::{general_error, internal_error, invalid_operation, not_found},
    p2p::services::mempool::MempoolHandle,
};

const LOG_TARGET: &str = "tari::validator_node::json_rpc::handlers";

pub struct JsonRpcHandlers {
    config: ApplicationConfig,
    keypair: RistrettoKeypair,
    mempool: MempoolHandle,
    template_provider: ValidatorTemplateProvider,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    layer_one_transaction_submitter: FileLayerOneSubmitter,
    global_db: GlobalDb<SqliteGlobalDbAdapter<PeerAddress>>,
    consensus: ConsensusHandle,
    consensus_constants: ConsensusConstants,
    networking: NetworkingHandle<TariMessagingSpec>,
    state_store: ValidatorNodeStateStore,
}

impl JsonRpcHandlers {
    pub fn new(services: &Services<ValidatorNodeStateStore>) -> Self {
        Self {
            config: services.config.clone(),
            keypair: services.keypair.clone(),
            mempool: services.mempool.clone(),
            template_provider: services.template_provider.clone(),
            epoch_manager: services.epoch_manager.clone(),
            consensus: services.consensus_handle.clone(),
            consensus_constants: services.consensus_constants.clone(),
            global_db: services.global_db.clone(),
            layer_one_transaction_submitter: services.layer_one_transaction_submitter.clone(),
            networking: services.networking.clone(),
            state_store: services.state_store.clone(),
        }
    }

    pub fn sidechain_id(&self) -> Option<&RistrettoPublicKey> {
        self.config.validator_node.sidechain_id.as_ref()
    }

    pub fn networking_is_active(&self) -> bool {
        !self.networking.is_closed()
    }

    pub fn consensus_status(&self) -> ConsensusCurrentState {
        self.consensus.get_current_state()
    }
}

impl JsonRpcHandlers {
    pub async fn get_identity(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let info = self
            .networking
            .get_local_peer_info()
            .await
            .map_err(internal_error(answer_id.clone()))?;
        let fee_claim_public_key = self.config.validator_node.fee_claim_public_key.to_byte_type();
        let response = GetIdentityResponse {
            peer_id: info.peer_id.to_string(),
            public_key: self.keypair.public_key().to_byte_type(),
            public_addresses: info.listen_addrs,
            supported_protocols: info.protocols.into_iter().map(|p| p.to_string()).collect(),
            protocol_version: info.protocol_version,
            user_agent: info.agent_version,
            fee_claim_public_key,
        };

        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn submit_transaction(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let SubmitTransactionRequest { transaction } = value.parse_params()?;
        let tx_id = transaction.calculate_id();
        debug!(
            target: LOG_TARGET,
            "Transaction {} has {} involved substate addresses",
            tx_id,
            transaction.involved_substate_addresses_iter().count()
        );

        if transaction.is_dry_run() {
            return Err(invalid_operation(
                answer_id.clone(),
                "Dry-run transactions cannot be submitted via this endpoint.",
            ));
        }

        // Submit to mempool.
        self.mempool.submit_transaction(transaction).await.map_err(|e| {
            if e.is_transaction_validator_error() {
                warn!(target: LOG_TARGET, "❌ Mempool rejected the transaction: {}", e);
                JsonRpcResponse::error(
                    answer_id.clone(),
                    JsonRpcError::new(
                        JsonRpcErrorReason::InvalidRequest,
                        format!("Mempool rejected the transaction: {}", e),
                        json::Value::Null,
                    ),
                )
            } else {
                error!(target: LOG_TARGET, "🚨 Mempool error: {}", e);
                JsonRpcResponse::error(
                    answer_id.clone(),
                    JsonRpcError::new(
                        JsonRpcErrorReason::InternalError,
                        format!("Mempool error: {}", e),
                        json::Value::Null,
                    ),
                )
            }
        })?;

        Ok(JsonRpcResponse::success(answer_id, SubmitTransactionResponse {
            transaction_id: tx_id,
            dry_run_result: None,
        }))
    }

    pub async fn get_state(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: GetStateRequest = value.parse_params()?;

        let tx = self.state_store.create_read_tx().unwrap();
        let state = SubstateRecord::get(&tx, &request.address)
            .optional()
            .map_err(internal_error(answer_id.clone()))?
            .ok_or_else(|| not_found(answer_id.clone(), format!("Substate {} not found", request.address)))?;
        let Some(substate) = state.into_substate() else {
            return Err(JsonRpcResponse::error(
                answer_id.clone(),
                JsonRpcError::new(
                    JsonRpcErrorReason::ApplicationError(100),
                    format!("Substate {} is DOWN", request.address),
                    json::Value::Null,
                ),
            ));
        };

        Ok(JsonRpcResponse::success(answer_id, GetStateResponse {
            data: substate.to_bytes(),
        }))
    }

    pub async fn list_blocks(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let req = value.parse_params::<ListBlocksRequest>()?;

        let current_epoch = self
            .epoch_manager
            .current_epoch()
            .await
            .map_err(internal_error(answer_id.clone()))?;

        let tx = self
            .state_store
            .create_read_tx()
            .map_err(internal_error(answer_id.clone()))?;

        let start_block = match req.from_id {
            Some(id) => Block::get(&tx, &id)
                .optional()
                .map_err(internal_error(answer_id.clone()))?
                .ok_or_else(|| not_found(answer_id.clone(), format!("Block {} not found", id)))?,
            None => {
                let leaf = LeafBlock::get(&tx, current_epoch)
                    .optional()
                    .map_err(internal_error(answer_id.clone()))?
                    .ok_or_else(|| not_found(answer_id.clone(), format!("No leaf block for epoch {current_epoch}")))?;
                Block::get(&tx, leaf.block_id()).map_err(internal_error(answer_id.clone()))?
            },
        };
        let blocks = tx
            .blocks_get_parent_chain(start_block.id(), req.limit)
            .map_err(internal_error(answer_id.clone()))?;

        let res = ListBlocksResponse { blocks };
        Ok(JsonRpcResponse::success(answer_id, res))
    }

    pub async fn get_tx_pool(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        if !self.consensus.get_current_state().is_running() {
            // Describe better why the following call may fail
            return Err(general_error(
                answer_id,
                "Consensus is not running. Please try again later",
            ));
        }
        let tx_pool = self
            .state_store
            .with_read_tx(|tx| tx.transaction_pool_get_all(1000))
            .map_err(internal_error(answer_id.clone()))?;
        let res = json!({ "tx_pool": tx_pool });
        Ok(JsonRpcResponse::success(answer_id, res))
    }

    pub async fn get_transaction_result(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: GetTransactionResultRequest = value.parse_params()?;

        let (execution, finalize_at) = self
            .state_store
            .with_read_tx(|tx| {
                let exec = TransactionExecution::get_finalized(tx, &request.transaction_id)?;
                let time = TransactionExecution::get_finalized_time(tx, &request.transaction_id)?;
                Ok::<_, StorageError>((exec, time))
            })
            .optional()
            .map_err(internal_error(answer_id.clone()))?
            .ok_or_else(|| {
                not_found(
                    answer_id.clone(),
                    format!("Transaction {} not found", request.transaction_id),
                )
            })?;

        let response = GetTransactionResultResponse {
            final_decision: Decision::from(&execution.result.finalize.result),
            transaction_execution: execution,
            finalize_at,
        };
        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_transaction(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: GetTransactionRequest = value.parse_params()?;

        let transaction = self
            .state_store
            .with_read_tx(|tx| TransactionRecord::get(tx, &data.transaction_id).optional())
            .map_err(internal_error(answer_id.clone()))?
            .ok_or_else(|| {
                not_found(
                    answer_id.clone(),
                    format!("Transaction {} not found", data.transaction_id),
                )
            })?;

        Ok(JsonRpcResponse::success(answer_id, GetTransactionResponse {
            transaction: transaction.into_transaction(),
        }))
    }

    pub async fn get_substate(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: GetSubstateRequest = value.parse_params()?;

        let maybe_substate = self
            .state_store
            .with_read_tx(|tx| {
                let address = SubstateAddress::from_substate_id(&data.address, data.version);
                SubstateRecord::get(tx, &address).optional()
            })
            .map_err(internal_error(answer_id.clone()))?;

        match maybe_substate {
            Some(substate) if substate.is_destroyed() => Ok(JsonRpcResponse::success(answer_id, GetSubstateResponse {
                status: SubstateStatus::Down,
                value: None,
            })),
            Some(substate) => Ok(JsonRpcResponse::success(answer_id, GetSubstateResponse {
                status: SubstateStatus::Up,
                value: substate.into_substate_value(),
            })),
            None => Ok(JsonRpcResponse::success(answer_id, GetSubstateResponse {
                status: SubstateStatus::DoesNotExist,
                value: None,
            })),
        }
    }

    pub async fn get_block(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let data: GetBlockRequest = value.parse_params()?;
        let (block, executions, total_block_execution_weight) = self
            .state_store
            .with_read_tx(|tx| {
                let Some(block) = Block::get(tx, &data.block_id).optional()? else {
                    return Ok(None);
                };
                let executions = tx.block_transaction_executions_get_all_for_block(&data.block_id)?;
                let total_block_execution_weight = sum_block_execution_weight(tx, &block)?;
                Ok::<_, StorageError>(Some((block, executions, total_block_execution_weight)))
            })
            .map_err(internal_error(answer_id.clone()))?
            .ok_or_else(|| not_found(answer_id.clone(), format!("Block {} not found", data.block_id)))?;

        let total_wasm_execution_points = executions
            .iter()
            .map(|e| e.result().wasm_execution_points)
            .fold(0u64, u64::saturating_add);
        let total_native_execution_points = executions
            .iter()
            .map(|e| e.result().native_execution_points)
            .fold(0u64, u64::saturating_add);

        let res = GetBlockResponse {
            block,
            total_wasm_execution_points,
            total_native_execution_points,
            max_block_execution_points: self.consensus_constants.max_block_execution_points,
            total_block_execution_weight,
            max_block_validation_weight: self.consensus_constants.max_block_validation_weight,
        };
        Ok(JsonRpcResponse::success(answer_id, res))
    }

    pub async fn get_filtered_blocks_count(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let req: GetFilteredBlocksCountRequest = value.parse_params()?;
        let count = self
            .state_store
            .with_read_tx(|tx| tx.filtered_blocks_get_count(req.filter_index, req.filter))
            .map_err(internal_error(answer_id.clone()))?;
        let res = GetBlocksCountResponse { count };
        Ok(JsonRpcResponse::success(answer_id, res))
    }

    pub async fn get_blocks(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let req: GetBlocksRequest = value.parse_params()?;
        // TODO: use a snapshot to prevent locking up the DB
        let blocks = self
            .state_store
            .with_read_tx(|tx| {
                tx.blocks_get_paginated(
                    req.limit.into(),
                    req.offset.into(),
                    req.filter_index,
                    req.filter,
                    req.ordering_index,
                    req.ordering,
                )
            })
            .map_err(internal_error(answer_id.clone()))?;
        let res = GetBlocksResponse { blocks };
        Ok(JsonRpcResponse::success(answer_id, res))
    }

    pub async fn get_template(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let req: GetTemplateRequest = value.parse_params()?;

        let loaded = self
            .template_provider
            .get_template(&req.template_address)
            .map_err(internal_error(answer_id.clone()))?
            .ok_or_else(|| {
                not_found(
                    answer_id.clone(),
                    format!("Template with address {} not found ", req.template_address),
                )
            })?;

        let abi = TemplateAbi {
            template_name: loaded.template_def().template_name().to_string(),
            functions: loaded
                .template_def()
                .functions()
                .iter()
                .map(|f| FunctionDef {
                    name: f.name.clone(),
                    arguments: f.arguments.to_vec(),
                    output: f.output.to_string(),
                    is_mut: f.is_mut,
                })
                .collect(),
            version: loaded.template_def().abi_version(),
        };

        Ok(JsonRpcResponse::success(answer_id, GetTemplateResponse {
            metadata: TemplateMetadata {
                name: loaded.template_name().to_string(),
                address: req.template_address,
                code_size: loaded.code_size(),
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
            .map_err(internal_error(answer_id.clone()))?;

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
                user_agent: conn.user_agent,
            })
            .collect();

        Ok(JsonRpcResponse::success(answer_id, GetConnectionsResponse {
            connections,
        }))
    }

    pub async fn get_mempool_stats(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let size = self
            .mempool
            .get_mempool_size()
            .await
            .map_err(internal_error(answer_id.clone()))?;
        Ok(JsonRpcResponse::success(answer_id, GetMempoolStatsResponse { size }))
    }

    pub async fn get_epoch_manager_stats(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        self.epoch_manager
            .wait_for_initial_scanning_to_complete()
            .await
            .map_err(internal_error(answer_id.clone()))?;
        let current_epoch = self.epoch_manager.get_current_epoch();
        let current_epoch_hash = self
            .epoch_manager
            .get_current_epoch_hash()
            .await
            .map_err(internal_error(answer_id.clone()))?;

        let current_block_height = self
            .global_db
            .create_transaction()
            .and_then(|mut tx| {
                self.global_db
                    .metadata(&mut tx)
                    .get_metadata::<u64>(StoreKey::BaseLayerLastScannedBlockHeight.as_key_bytes())
            })
            .map_err(|e| {
                JsonRpcResponse::error(
                    answer_id.clone(),
                    JsonRpcError::new(
                        JsonRpcErrorReason::InternalError,
                        format!("Could not get current block height: {}", e),
                        json::Value::Null,
                    ),
                )
            })?;
        let local_vn_start_epoch = self
            .epoch_manager
            .get_our_validator_node(current_epoch)
            .await
            .map(|vn| vn.start_epoch)
            .map(Some)
            .or_else(|err| {
                if err.is_not_registered_error() {
                    Ok(None)
                } else {
                    Err(JsonRpcResponse::error(
                        answer_id.clone(),
                        JsonRpcError::new(
                            JsonRpcErrorReason::InternalError,
                            format!("Could not get committee shard:{}", err),
                            json::Value::Null,
                        ),
                    ))
                }
            })?;
        let committee_info = self
            .epoch_manager
            .get_local_committee_info(current_epoch)
            .await
            .map(Some)
            .or_else(|err| {
                if err.is_not_registered_error() {
                    Ok(None)
                } else {
                    Err(JsonRpcResponse::error(
                        answer_id.clone(),
                        JsonRpcError::new(
                            JsonRpcErrorReason::InternalError,
                            format!("Could not get committee shard:{}", err),
                            json::Value::Null,
                        ),
                    ))
                }
            })?;
        let is_initial_scanning_complete = self
            .epoch_manager
            .is_initial_scanning_complete()
            .await
            .map_err(internal_error(answer_id.clone()))?;
        let response = GetEpochManagerStatsResponse {
            current_epoch,
            current_block_height: current_block_height.unwrap_or(0),
            current_block_hash: current_epoch_hash,
            is_initial_scanning_complete,
            is_valid: committee_info.is_some(),
            start_epoch: local_vn_start_epoch,
            committee_info,
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

        let Ok(public_key) = RistrettoPublicKey::convert_from_byte_type(&public_key) else {
            return Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    "Invalid public key".to_string(),
                    json::Value::Null,
                ),
            ));
        };

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

        if *self.networking.local_peer_id() == peer_id {
            return if wait_for_dial {
                Err(JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(
                        JsonRpcErrorReason::InvalidParams,
                        "Cannot add self as peer".to_string(),
                        json::Value::Null,
                    ),
                ))
            } else {
                Ok(JsonRpcResponse::success(answer_id, AddPeerResponse {}))
            };
        }

        let dial_wait = networking
            .dial_peer(
                DialOpts::peer_id(peer_id)
                    .addresses(addresses)
                    .condition(PeerCondition::Always)
                    .build(),
            )
            .await
            .map_err(internal_error(answer_id.clone()))?;

        if wait_for_dial {
            dial_wait.await.map_err(internal_error(answer_id.clone()))?;
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
            .map_err(internal_error(answer_id.clone()))?;

        let status = if peers.is_empty() { "Offline" } else { "Online" };
        Ok(JsonRpcResponse::success(answer_id, GetCommsStatsResponse {
            connection_status: status.to_string(),
        }))
    }

    pub async fn get_shard_key(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request = value.parse_params::<GetShardKeyRequest>()?;
        let maybe_vn = self
            .epoch_manager
            .get_our_validator_node(request.epoch)
            .await
            .optional()
            .map_err(internal_error(answer_id.clone()))?;

        Ok(JsonRpcResponse::success(answer_id, GetShardKeyResponse {
            shard_key: maybe_vn.map(|vn| vn.shard_key),
        }))
    }

    pub async fn get_committee(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request = value.parse_params::<GetCommitteeRequest>()?;
        let committee = self
            .epoch_manager
            .get_committee_for_substate(request.epoch, request.substate_address)
            .await
            .map_err(internal_error(answer_id.clone()))?;

        Ok(JsonRpcResponse::success(answer_id, GetCommitteeResponse { committee }))
    }

    pub async fn get_all_vns(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let GetAllVnsRequest { epoch } = value.parse_params::<GetAllVnsRequest>()?;

        let vns = self
            .epoch_manager
            .get_all_validator_nodes(epoch)
            .await
            .map_err(internal_error(answer_id.clone()))?;

        let vns = vns
            .into_iter()
            .map(|vn| BaseLayerValidatorNode {
                public_key: vn.public_key,
                shard_key: vn.shard_key,
                sidechain_id: None,
            })
            .collect();

        Ok(JsonRpcResponse::success(answer_id, GetAllVnsResponse { vns }))
    }

    pub async fn get_consensus_status(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let epoch = self.consensus.current_epoch();
        let committee_info = self
            .epoch_manager
            .get_local_committee_info(epoch)
            .await
            // None when the VN is not registered or the epoch is not yet established (NoEpochFound
            // during startup); a genuine error otherwise.
            .optional()
            .map_err(internal_error(answer_id.clone()))?;
        let height = self.consensus.current_view().get_height();
        let state = self.consensus.get_current_state();
        let state_versions = committee_info
            .map(|committee_info| {
                self.state_store
                    .with_read_tx(|tx| tx.state_tree_versions_get_latest_for_shard_group(committee_info.shard_group()))
                    .map_err(internal_error(answer_id.clone()))
                    .map(|sv| sv.convert_to_map(committee_info.shard_group()))
            })
            .transpose()?;

        Ok(JsonRpcResponse::success(answer_id, GetConsensusStatusResponse {
            epoch,
            height,
            state: state.to_string(),
            state_versions,
        }))
    }

    #[allow(clippy::too_many_lines)]
    pub async fn prepare_layer_one_transaction(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request = value.parse_params::<types::PrepareLayerOneTransactionRequest>()?;
        let path = match request.params {
            LayerOneTransactionParams::Registration => {
                let current_epoch = self
                    .epoch_manager
                    .current_epoch()
                    .await
                    .map_err(internal_error(answer_id.clone()))?;
                if self
                    .epoch_manager
                    .is_this_validator_registered_for_epoch(current_epoch)
                    .await
                    .map_err(internal_error(answer_id.clone()))?
                {
                    return Err(invalid_operation(
                        answer_id,
                        "Cannot submit registration for validator node that is already registered",
                    ));
                }

                // Use the currently configured fee claim public key
                let fee_claim_public_key = self.config.validator_node.fee_claim_public_key.to_byte_type();
                let sidechain_id = self.sidechain_id().map(|s| {
                    // The following is guaranteed to be infallible (invariant), more of a shortcoming of the
                    // CompressedPublicKey API. new_from_pk requires a clone
                    CompressedPublicKey::from_canonical_bytes(s.as_bytes()).expect(
                        "INVARIANT VIOLATION: \
                         CompressedPublicKey::from_canonical_bytes(fee_claim_public_key.as_bytes()) returned an error",
                    )
                });
                // TODO: we permit the registration within the next 3 epochs. Since we scan behind the chain, this
                // should be set according to the number of epochs behind (consensus constants) rather than arbitrarily.
                let max_epoch = current_epoch + Epoch(3);

                let signature = ValidatorNodeSignature::sign_for_registration(
                    self.keypair.secret_key(),
                    sidechain_id.as_ref(),
                    &CompressedPublicKey::from_canonical_bytes(fee_claim_public_key.as_bytes()).expect(
                        "INVARIANT VIOLATION: \
                         CompressedPublicKey::from_canonical_bytes(fee_claim_public_key.as_bytes()) returned an error",
                    ),
                    max_epoch.as_u64().into(),
                );
                let l1_tx = LayerOneTransactionDef {
                    payload_type: LayerOnePayloadType::ValidatorRegistration,
                    payload: ValidatorRegistrationParams {
                        public_key: self.keypair.public_key().to_byte_type(),
                        signature: SchnorrSignatureBytes::new(
                            RistrettoPublicKeyBytes::from_bytes(
                                signature.signature().get_compressed_public_nonce().as_bytes(),
                            )
                            .expect("INVARIANT VIOLATION: ristretto public key length mismatch"),
                            Scalar32Bytes::from_bytes(signature.signature().get_signature().as_bytes())
                                .expect("INVARIANT VIOLATION: signature scalar length mismatch"),
                        ),
                        claim_public_key: fee_claim_public_key,
                        max_epoch,
                        // TODO: this wont work if Some because the wallet expects a private key - who should hold the
                        // sidechain secret key? This depends on the required security model.
                        // Likely the Minotari wallet, which implies functionality that is able
                        // to "look up" the secret from the public key provided here.
                        sidechain_public_key: self.sidechain_id().map(|pk| pk.to_byte_type()),
                    },
                };
                self.layer_one_transaction_submitter
                    .submit_transaction(l1_tx)
                    .await
                    .map_err(internal_error(answer_id.clone()))?
            },
            LayerOneTransactionParams::Exit => {
                let current_epoch = self
                    .epoch_manager
                    .current_epoch()
                    .await
                    .map_err(internal_error(answer_id.clone()))?;
                if !self
                    .epoch_manager
                    .is_this_validator_registered_for_epoch(current_epoch)
                    .await
                    .map_err(internal_error(answer_id.clone()))?
                {
                    return Err(invalid_operation(
                        answer_id,
                        "Cannot submit exit for validator node that is  not registered",
                    ));
                }

                let max_epoch = current_epoch + Epoch(3);
                let signature = ValidatorNodeSignature::sign_for_exit(
                    self.keypair.secret_key(),
                    // TODO: sidechain support
                    None,
                    max_epoch.as_u64().into(),
                );
                let l1_tx = LayerOneTransactionDef {
                    payload_type: LayerOnePayloadType::ValidatorExit,
                    payload: ValidatorExitParams {
                        public_key: self.keypair.public_key().to_byte_type(),
                        signature: SchnorrSignatureBytes::new(
                            RistrettoPublicKeyBytes::from_bytes(
                                signature.signature().get_compressed_public_nonce().as_bytes(),
                            )
                            .expect("INVARIANT VIOLATION: ristretto public key length mismatch"),
                            Scalar32Bytes::from_bytes(signature.signature().get_signature().as_bytes())
                                .expect("INVARIANT VIOLATION: signature scalar length mismatch"),
                        ),
                        max_epoch,
                        sidechain_public_key: self.sidechain_id().map(|pk| pk.to_byte_type()),
                    },
                };
                self.layer_one_transaction_submitter
                    .submit_transaction(l1_tx)
                    .await
                    .map_err(internal_error(answer_id.clone()))?
            },
        };

        Ok(JsonRpcResponse::success(
            answer_id,
            types::PrepareLayerOneTransactionResponse { path },
        ))
    }
}

/// Sums the execution weight of the block's transaction commands, expression for expression as a replica does when
/// deciding whether to vote for the block: a transaction's weight, discounted by how much of the work its command
/// stage actually adds. The result belongs with `max_block_validation_weight` and no other budget — a leader packs
/// against a lower one under a different rule, spending it on foreign proposals too.
///
/// Two consequences of following the validation rule: a command carrying no execution weight (a foreign proposal)
/// contributes nothing, and the discount floors where the proposer's rounds up, so a light transaction at a
/// discounted stage can contribute 0 where the leader charged 1.
///
/// Transactions pruned from storage contribute nothing, so an old block's total may read low.
fn sum_block_execution_weight<TTx: StateStoreReadTransaction>(tx: &TTx, block: &Block) -> Result<u64, StorageError> {
    let mut total = 0u64;
    for cmd in block.commands() {
        let Some(atom) = cmd.transaction() else {
            continue;
        };
        let Some(record) = atom.get_transaction(tx).optional()? else {
            continue;
        };
        let weight = record.transaction().calculate_transaction_weight().as_u64();
        total = total.saturating_add(weight.saturating_mul(cmd.execution_weight_percent()) / 100);
    }
    Ok(total)
}
