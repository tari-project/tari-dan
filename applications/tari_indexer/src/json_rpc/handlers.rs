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

use std::{convert::TryInto, str::FromStr};

use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use futures::StreamExt;
use log::info;
use serde::Serialize;
use serde_json::{self as json, json};
use tari_comms::CommsNode;
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_dan_common_types::{Epoch, ShardId, SubstateState};
use tari_dan_core::services::{epoch_manager::EpochManager, BaseNodeClient, ValidatorNodeClientFactory};
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStore;
use tari_engine_types::substate::{Substate, SubstateAddress};

use crate::{
    bootstrap::Services,
    p2p::{
        proto::rpc::VnStateSyncResponse,
        services::{epoch_manager::handle::EpochManagerHandle, rpc_client::TariCommsValidatorNodeClientFactory},
    },
    GrpcBaseNodeClient,
};

const LOG_TARGET: &str = "tari::indexer::json_rpc::handlers";

pub struct JsonRpcHandlers {
    addresses: Vec<SubstateAddress>,
    comms: CommsNode,
    epoch_manager: EpochManagerHandle,
    _shard_store: SqliteShardStore,
    base_node_client: GrpcBaseNodeClient,
    validator_node_client_factory: TariCommsValidatorNodeClientFactory,
}

impl JsonRpcHandlers {
    pub fn new(addresses: Vec<SubstateAddress>, services: &Services, base_node_client: GrpcBaseNodeClient) -> Self {
        Self {
            addresses,
            comms: services.comms.clone(),
            epoch_manager: services.epoch_manager.clone(),
            _shard_store: services.shard_store.clone(),
            base_node_client,
            validator_node_client_factory: services.validator_node_client_factory.clone(),
        }
    }
}

impl JsonRpcHandlers {
    pub fn get_status(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let response = GetStatusResponse {
            addresses: self.addresses.iter().map(SubstateAddress::to_string).collect(),
        };

        Ok(JsonRpcResponse::success(answer_id, response))
    }

    pub async fn get_all_vns(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let epoch: u64 = value.parse_params()?;
        if let Ok(vns) = self.base_node_client.clone().get_validator_nodes(epoch * 10).await {
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

    pub async fn get_substate(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let substate_address_str: String = value.parse_params()?;
        let substate_address = SubstateAddress::from_str(&substate_address_str).unwrap();
        let epoch = self.epoch_manager.current_epoch().await.unwrap();

        let mut latest_substate = None;
        let mut version = 0;

        // we keep asking from version 0 upwards until a version does not exist
        while let Some(substate) = self.get_substate_from_commitee(&substate_address, version, epoch).await {
            latest_substate = Some(substate);
            version += 1;
        }

        if let Some(substate) = latest_substate {
            return Ok(JsonRpcResponse::success(answer_id, substate));
        }

        Err(JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(
                JsonRpcErrorReason::InvalidParams,
                "Could not retrieve the substate from the network".to_string(),
                json::Value::Null,
            ),
        ))
    }

    async fn get_substate_from_commitee(
        &self,
        substate_address: &SubstateAddress,
        version: u32,
        epoch: Epoch,
    ) -> Option<Substate> {
        let shard_id = ShardId::from_address(substate_address, version);
        let committee = self.epoch_manager.get_committee(epoch, shard_id).await.unwrap();

        for vn_public_key in &committee.members {
            let res = self.get_substate_state_from_vn(vn_public_key, shard_id, version).await;

            if let Ok(SubstateState::Up { data: substate, .. }) = res {
                return Some(substate);
            }
        }

        None
    }

    async fn get_substate_state_from_vn(
        &self,
        vn_public_key: &RistrettoPublicKey,
        shard_id: ShardId,
        _version: u32,
    ) -> Result<SubstateState, anyhow::Error> {
        // build a client with the VN
        let mut sync_vn_client = self.validator_node_client_factory.create_client(vn_public_key);
        let mut sync_vn_rpc_client = sync_vn_client.create_connection().await?;

        // request the shard substate to the VN
        let shard_id_proto: crate::p2p::proto::common::ShardId = shard_id.into();
        let request = crate::p2p::proto::rpc::VnStateSyncRequest {
            start_shard_id: Some(shard_id_proto.clone()),
            end_shard_id: Some(shard_id_proto),
            inventory: vec![],
        };

        // get the VN's response
        let mut vn_state_stream = match sync_vn_rpc_client.vn_state_sync(request).await {
            Ok(stream) => stream,
            Err(e) => {
                info!(target: LOG_TARGET, "Unable to connect to peer: {} ", e);
                return Err(e.into());
            },
        };

        // return the substate from the response (there is going to be only 0 or 1 result)
        if let Some(msg) = vn_state_stream.next().await {
            let msg = msg?;
            let state = extract_state_from_vn_response(msg)?;
            return Ok(state);
        }

        Ok(SubstateState::DoesNotExist)
    }
}

fn extract_state_from_vn_response(msg: VnStateSyncResponse) -> Result<SubstateState, anyhow::Error> {
    let state = if let Some(deleted_by) = Some(msg.destroyed_payload_id).filter(|p| !p.is_empty()) {
        SubstateState::Down {
            deleted_by: deleted_by.try_into()?,
        }
    } else {
        let substate = Substate::from_bytes(&msg.substate)?;
        SubstateState::Up {
            created_by: msg.created_payload_id.try_into()?,
            data: substate,
        }
    };

    Ok(state)
}

#[derive(Serialize, Debug)]
struct GetStatusResponse {
    addresses: Vec<String>,
}
