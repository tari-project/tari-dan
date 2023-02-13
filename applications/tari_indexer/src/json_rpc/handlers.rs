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

use crate::{bootstrap::Services, dan_layer_scanner::DanLayerScanner, GrpcBaseNodeClient};

pub struct JsonRpcHandlers {
    comms: CommsNode,
    base_node_client: GrpcBaseNodeClient,
    dan_layer_scanner: Arc<DanLayerScanner>,
}

impl JsonRpcHandlers {
    pub fn new(
        services: &Services,
        base_node_client: GrpcBaseNodeClient,
        dan_layer_scanner: Arc<DanLayerScanner>,
    ) -> Self {
        Self {
            comms: services.comms.clone(),
            base_node_client,
            dan_layer_scanner,
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
        let request: GetSubstateRequest = value.parse_params()?;
        let substate_address_str: String = request.address;
        let substate_address = SubstateAddress::from_str(&substate_address_str).unwrap();

        match self
            .dan_layer_scanner
            .get_substate(substate_address, request.version)
            .await
        {
            Some(substate) => Ok(JsonRpcResponse::success(answer_id, substate)),
            None => Err(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    "Could not retrieve the substate from the network".to_string(),
                    json::Value::Null,
                ),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubstateRequest {
    pub address: String,
    pub version: Option<u32>,
}
