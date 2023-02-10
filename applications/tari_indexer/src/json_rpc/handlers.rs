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

use std::str::FromStr;

use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use serde::Serialize;
use serde_json::{self as json, json};
use tari_comms::CommsNode;
use tari_dan_common_types::ShardId;
use tari_dan_core::services::{epoch_manager::EpochManager, BaseNodeClient};
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStore;
use tari_engine_types::substate::SubstateAddress;

use crate::{bootstrap::Services, p2p::services::epoch_manager::handle::EpochManagerHandle, GrpcBaseNodeClient};

const _LOG_TARGET: &str = "tari::indexer::json_rpc::handlers";

pub struct JsonRpcHandlers {
    addresses: Vec<SubstateAddress>,
    comms: CommsNode,
    epoch_manager: EpochManagerHandle,
    _shard_store: SqliteShardStore,
    base_node_client: GrpcBaseNodeClient,
}

impl JsonRpcHandlers {
    pub fn new(addresses: Vec<SubstateAddress>, services: &Services, base_node_client: GrpcBaseNodeClient) -> Self {
        Self {
            addresses,
            comms: services.comms.clone(),
            epoch_manager: services.epoch_manager.clone(),
            _shard_store: services.shard_store.clone(),
            base_node_client,
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
        let version = 0;
        let shard_id = ShardId::from_address(&substate_address, version);

        let epoch = self.epoch_manager.current_epoch().await.unwrap();
        let response = self.epoch_manager.get_committee(epoch, shard_id).await.unwrap();

        Ok(JsonRpcResponse::success(answer_id, response))
    }
}

#[derive(Serialize, Debug)]
struct GetStatusResponse {
    addresses: Vec<String>,
}
