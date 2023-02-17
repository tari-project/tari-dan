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

use std::{str::FromStr, sync::Arc};

use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::{self as json, json};
use tari_comms::CommsNode;
use tari_dan_core::services::BaseNodeClient;
use tari_engine_types::substate::SubstateAddress;

use crate::{bootstrap::Services, substate_manager::SubstateManager, GrpcBaseNodeClient};

pub struct JsonRpcHandlers {
    comms: CommsNode,
    base_node_client: GrpcBaseNodeClient,
    substate_manager: Arc<SubstateManager>,
}

impl JsonRpcHandlers {
    pub fn new(
        services: &Services,
        base_node_client: GrpcBaseNodeClient,
        substate_manager: Arc<SubstateManager>,
    ) -> Self {
        Self {
            comms: services.comms.clone(),
            base_node_client,
            substate_manager,
        }
    }
}

impl JsonRpcHandlers {
    pub async fn get_all_vns(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let epoch: u64 = value.parse_params()?;
        if let Ok(vns) = self.base_node_client.clone().get_validator_nodes(epoch * 10).await {
            let response = json!({ "vns": vns });
            Ok(JsonRpcResponse::success(answer_id, response))
        } else {
            Err(Self::generic_error_response(answer_id))
        }
    }

    pub async fn get_comms_stats(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        if let Ok(stats) = self.comms.connectivity().get_connectivity_status().await {
            let response = json!({ "connection_status": format!("{:?}", stats) });
            Ok(JsonRpcResponse::success(answer_id, response))
        } else {
            Err(Self::generic_error_response(answer_id))
        }
    }

    pub async fn get_substate(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: GetSubstateRequest = value.parse_params()?;
        let substate_address = Self::parse_substate_address(&request.address, answer_id)?;
        let version = request.version;

        let res = self
            .substate_manager
            .get_substate(&substate_address, version)
            .await
            .unwrap_or(None);

        match res {
            Some(substate) => Ok(JsonRpcResponse::success(answer_id, substate)),
            None => Err(Self::generic_error_response(answer_id)),
        }
    }

    pub async fn get_addresses(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();

        let res = self.substate_manager.get_all_addresses_from_db().await;

        match res {
            Ok(addresses) => Ok(JsonRpcResponse::success(answer_id, addresses)),
            Err(_) => Err(Self::generic_error_response(answer_id)),
        }
    }

    pub async fn add_address(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: AddAddressRequest = value.parse_params()?;
        let substate_address = Self::parse_substate_address(&request.address, answer_id)?;

        match self
            .substate_manager
            .fetch_and_add_substate_to_db(&substate_address)
            .await
        {
            Ok(_) => Ok(JsonRpcResponse::success(answer_id, ())),
            Err(_) => Err(Self::generic_error_response(answer_id)),
        }
    }

    pub async fn delete_address(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: DeleteAddressRequest = value.parse_params()?;
        let substate_address = Self::parse_substate_address(&request.address, answer_id)?;

        match self.substate_manager.delete_substate_from_db(&substate_address).await {
            Ok(_) => Ok(JsonRpcResponse::success(answer_id, ())),
            Err(_) => Err(Self::generic_error_response(answer_id)),
        }
    }

    pub async fn clear_addresses(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();

        match self.substate_manager.delete_all_substates_from_db().await {
            Ok(_) => Ok(JsonRpcResponse::success(answer_id, ())),
            Err(_) => Err(Self::generic_error_response(answer_id)),
        }
    }

    fn parse_substate_address(address_str: &str, answer_id: i64) -> Result<SubstateAddress, JsonRpcResponse> {
        let address = SubstateAddress::from_str(address_str).map_err(|_| Self::generic_error_response(answer_id))?;
        Ok(address)
    }

    fn generic_error_response(answer_id: i64) -> JsonRpcResponse {
        JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(
                JsonRpcErrorReason::InvalidParams,
                "Something went wrong".to_string(),
                json::Value::Null,
            ),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubstateRequest {
    pub address: String,
    pub version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddAddressRequest {
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteAddressRequest {
    pub address: String,
}
