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
mod error;
pub use error::ValidatorNodeClientError;
pub mod types;

use reqwest::{header, header::HeaderMap, IntoUrl, Url};
use serde::{de::DeserializeOwned, Serialize};
use serde_json as json;
use serde_json::json;
use tari_common_types::{transaction::TxId, types::PublicKey};
use tari_comms_logging::LoggedMessage;

use crate::types::{
    AddPeerRequest,
    AddPeerResponse,
    GetEpochManagerStatsResponse,
    GetIdentityResponse,
    GetRecentTransactionsRequest,
    GetRecentTransactionsResponse,
    GetStateRequest,
    GetStateResponse,
    GetSubstateRequest,
    GetSubstateResponse,
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
    RegisterValidatorNodeRequest,
    RegisterValidatorNodeResponse,
    SubmitTransactionRequest,
    SubmitTransactionResponse,
    TemplateRegistrationRequest,
    TemplateRegistrationResponse,
};

#[derive(Debug, Clone)]
pub struct ValidatorNodeClient {
    client: reqwest::Client,
    endpoint: Url,
    request_id: i64,
}

impl ValidatorNodeClient {
    pub fn connect<T: IntoUrl>(endpoint: T) -> Result<Self, ValidatorNodeClientError> {
        let client = reqwest::Client::builder()
            .default_headers({
                let mut headers = HeaderMap::with_capacity(1);
                headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
                headers
            })
            .build()?;

        Ok(Self {
            client,
            endpoint: endpoint.into_url()?,
            request_id: 0,
        })
    }

    pub async fn get_identity(&mut self) -> Result<GetIdentityResponse, ValidatorNodeClientError> {
        self.send_request("get_identity", json!({})).await
    }

    pub async fn get_epoch_manager_stats(&mut self) -> Result<GetEpochManagerStatsResponse, ValidatorNodeClientError> {
        self.send_request("get_epoch_manager_stats", json!({})).await
    }

    pub async fn register_validator_node(
        &mut self,
        claim_public_key: PublicKey,
    ) -> Result<TxId, ValidatorNodeClientError> {
        let resp: RegisterValidatorNodeResponse = self
            .send_request("register_validator_node", RegisterValidatorNodeRequest {
                fee_claim_public_key: claim_public_key,
            })
            .await?;
        Ok(resp.transaction_id)
    }

    pub async fn register_template(
        &mut self,
        request: TemplateRegistrationRequest,
    ) -> Result<TemplateRegistrationResponse, ValidatorNodeClientError> {
        self.send_request("register_template", request).await
    }

    pub async fn get_active_templates(
        &mut self,
        request: GetTemplatesRequest,
    ) -> Result<GetTemplatesResponse, ValidatorNodeClientError> {
        self.send_request("get_templates", request).await
    }

    pub async fn get_state(&mut self, request: GetStateRequest) -> Result<GetStateResponse, ValidatorNodeClientError> {
        self.send_request("get_state", request).await
    }

    pub async fn get_substate(
        &mut self,
        request: GetSubstateRequest,
    ) -> Result<GetSubstateResponse, ValidatorNodeClientError> {
        self.send_request("get_substate", request).await
    }

    pub async fn get_fees(
        &mut self,
        request: GetValidatorFeesRequest,
    ) -> Result<GetValidatorFeesResponse, ValidatorNodeClientError> {
        self.send_request("get_fees", request).await
    }

    pub async fn get_template(
        &mut self,
        request: GetTemplateRequest,
    ) -> Result<GetTemplateResponse, ValidatorNodeClientError> {
        self.send_request("get_template", request).await
    }

    pub async fn get_transaction(
        &mut self,
        request: GetTransactionRequest,
    ) -> Result<GetTransactionResponse, ValidatorNodeClientError> {
        self.send_request("get_transaction", request).await
    }

    pub async fn get_transaction_result(
        &mut self,
        request: GetTransactionResultRequest,
    ) -> Result<GetTransactionResultResponse, ValidatorNodeClientError> {
        self.send_request("get_transaction_result", request).await
    }

    pub async fn get_recent_transactions(
        &mut self,
        request: GetRecentTransactionsRequest,
    ) -> Result<GetRecentTransactionsResponse, ValidatorNodeClientError> {
        self.send_request("get_recent_transactions", request).await
    }

    pub async fn submit_transaction(
        &mut self,
        request: SubmitTransactionRequest,
    ) -> Result<SubmitTransactionResponse, ValidatorNodeClientError> {
        self.send_request("submit_transaction", request).await
    }

    pub async fn add_peer(&mut self, request: AddPeerRequest) -> Result<AddPeerResponse, ValidatorNodeClientError> {
        self.send_request("add_peer", request).await
    }

    pub async fn get_message_logs(
        &mut self,
        message_tag: &str,
    ) -> Result<Vec<LoggedMessage>, ValidatorNodeClientError> {
        let resp = self
            .send_request::<_, json::Value>("get_logged_messages", json!({ "message_tag": message_tag }))
            .await?;
        let messages = json::from_value(resp.get("messages").cloned().ok_or_else(|| {
            ValidatorNodeClientError::InvalidResponse {
                message: "messages was not provided".to_string(),
            }
        })?)
        .map_err(|e| ValidatorNodeClientError::DeserializeResponse {
            source: e,
            method: "get_logged_messages".to_string(),
        })?;
        Ok(messages)
    }

    fn next_request_id(&mut self) -> i64 {
        self.request_id += 1;
        self.request_id
    }

    async fn send_request<T: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: T,
    ) -> Result<R, ValidatorNodeClientError> {
        let params = json::to_value(params).map_err(|e| ValidatorNodeClientError::SerializeRequest {
            source: e,
            method: method.to_string(),
        })?;
        let request_json = json!(
            {
                "jsonrpc": "2.0",
                "id": self.next_request_id(),
                "method": method,
                "params": params,
            }
        );
        let resp = self
            .client
            .post(self.endpoint.clone())
            .body(request_json.to_string())
            .send()
            .await?;
        let val = resp.json().await?;
        let resp = jsonrpc_result(val)?;
        // Response might not deserialize to R....
        match serde_json::from_value(resp) {
            Ok(r) => Ok(r),
            Err(e) => Err(ValidatorNodeClientError::DeserializeResponse {
                source: e,
                method: method.to_string(),
            }),
        }
    }
}

fn jsonrpc_result(val: json::Value) -> Result<json::Value, ValidatorNodeClientError> {
    if let Some(err) = val.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
        return Err(ValidatorNodeClientError::RequestFailedWithStatus {
            code,
            message: message.to_string(),
        });
    }

    let result = val
        .get("result")
        .ok_or_else(|| ValidatorNodeClientError::InvalidResponse {
            message: "Missing result field".to_string(),
        })?;
    Ok(result.clone())
}
