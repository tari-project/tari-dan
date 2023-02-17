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

use crate::{
    bootstrap::Services,
    dan_layer_scanner::DanLayerScanner,
    substate_storage_sqlite::{
        models::substate::NewSubstate,
        sqlite_substate_store_factory::{
            SqliteSubstateStore,
            SubstateStore,
            SubstateStoreReadTransaction,
            SubstateStoreWriteTransaction,
        },
    },
    GrpcBaseNodeClient,
};

pub struct JsonRpcHandlers {
    comms: CommsNode,
    base_node_client: GrpcBaseNodeClient,
    dan_layer_scanner: Arc<DanLayerScanner>,
    substate_store: SqliteSubstateStore,
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
            substate_store: services.substate_store.clone(),
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
        let substate_address_str: String = request.address;
        let substate_address = SubstateAddress::from_str(&substate_address_str).unwrap();

        match self
            .dan_layer_scanner
            .get_substate(substate_address, request.version)
            .await
        {
            Some(substate) => Ok(JsonRpcResponse::success(answer_id, substate)),
            None => Err(Self::generic_error_response(answer_id)),
        }
    }

    pub async fn get_addresses(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let tx = self.substate_store.create_read_tx().unwrap();
        if let Ok(addresses) = tx.get_all_addresses() {
            Ok(JsonRpcResponse::success(answer_id, addresses))
        } else {
            Err(Self::generic_error_response(answer_id))
        }
    }

    pub async fn add_address(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: AddAddressRequest = value.parse_params()?;
        let substate_address_str: String = request.address;
        let substate_address = SubstateAddress::from_str(&substate_address_str).unwrap();

        // get the last version of the substate from the dan layer
        let substate_scan_result = self.dan_layer_scanner.get_substate(substate_address, None).await;

        // store the substate into the database
        if let Some(substate) = substate_scan_result {
            let pretty_data = serde_json::to_string_pretty(&substate).unwrap();
            let mut tx = self.substate_store.create_write_tx().unwrap();
            let substate_row = NewSubstate {
                address: substate_address_str,
                version: i64::from(substate.version()),
                data: pretty_data,
            };
            // if the substate is already stored it will be updated with the new version and data,
            // otherwise it will be inserted as a new row
            if tx.set_substate(substate_row).is_ok() {
                return Ok(JsonRpcResponse::success(answer_id, ()));
            }
        }

        Err(Self::generic_error_response(answer_id))
    }

    pub async fn delete_address(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        let request: DeleteAddressRequest = value.parse_params()?;

        let mut tx = self.substate_store.create_write_tx().unwrap();
        if tx.delete_substate(request.address).is_ok() {
            return Ok(JsonRpcResponse::success(answer_id, ()));
        }

        Err(Self::generic_error_response(answer_id))
    }

    pub async fn clear_addresses(&self, value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();

        let mut tx = self.substate_store.create_write_tx().unwrap();
        if tx.clear_substates().is_ok() {
            return Ok(JsonRpcResponse::success(answer_id, ()));
        }

        Err(Self::generic_error_response(answer_id))
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
