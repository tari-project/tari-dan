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

use reqwest::{header, header::HeaderMap, IntoUrl, Url};
use serde::{de::DeserializeOwned, Serialize};
use serde_json as json;
use serde_json::json;
use tari_engine_types::substate::SubstateAddress;

use crate::{
    error::IndexerClientError,
    types::{
        AddAddressRequest, DeleteAddressRequest, GetNonFungiblesRequest, GetNonFungiblesResponse, GetSubstateRequest,
        GetSubstateResponse, GetTransactionResultRequest, GetTransactionResultResponse, SubmitTransactionRequest,
        SubmitTransactionResponse,
    },
};

#[derive(Debug, Clone)]
pub struct IndexerJsonRpcClient {
    client: reqwest::Client,
    endpoint: Url,
    request_id: i64,
}

impl IndexerJsonRpcClient {
    pub fn connect<T: IntoUrl>(endpoint: T) -> Result<Self, IndexerClientError> {
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

    fn next_request_id(&mut self) -> i64 {
        self.request_id += 1;
        self.request_id
    }

    pub async fn add_address(&mut self, address: SubstateAddress) -> Result<(), IndexerClientError> {
        self.send_request("add_address", AddAddressRequest { address }).await
    }

    pub async fn get_substate(&mut self, req: GetSubstateRequest) -> Result<GetSubstateResponse, IndexerClientError> {
        self.send_request("get_substate", req).await
    }

    pub async fn submit_transaction(
        &mut self,
        req: SubmitTransactionRequest,
    ) -> Result<SubmitTransactionResponse, IndexerClientError> {
        self.send_request("submit_transaction", req).await
    }

    pub async fn get_transaction_result(
        &mut self,
        req: GetTransactionResultRequest,
    ) -> Result<GetTransactionResultResponse, IndexerClientError> {
        self.send_request("get_transaction_result", req).await
    }

    pub async fn delete_address(&mut self, req: DeleteAddressRequest) -> Result<(), IndexerClientError> {
        self.send_request("delete_address", req).await
    }

    pub async fn get_non_fungibles(
        &mut self,
        req: GetNonFungiblesRequest,
    ) -> Result<GetNonFungiblesResponse, IndexerClientError> {
        self.send_request("get_non_fungibles", req).await
    }

    async fn send_request<T: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: T,
    ) -> Result<R, IndexerClientError> {
        let params = json::to_value(params).map_err(|e| IndexerClientError::SerializeRequest {
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
            Err(e) => Err(IndexerClientError::DeserializeResponse {
                method: method.to_string(),
                source: e,
            }),
        }
    }
}

fn jsonrpc_result(val: json::Value) -> Result<json::Value, IndexerClientError> {
    if let Some(err) = val.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
        return Err(IndexerClientError::RequestFailedWithStatus {
            code,
            message: message.to_string(),
        });
    }

    let result = val.get("result").ok_or_else(|| IndexerClientError::InvalidResponse {
        message: "Missing result field".to_string(),
    })?;
    Ok(result.clone())
}
