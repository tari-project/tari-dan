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
pub mod error;
pub mod types;

use std::borrow::Borrow;

use reqwest::{header, header::HeaderMap, IntoUrl, Url};
use serde::{de::DeserializeOwned, Serialize};
use serde_json as json;
use serde_json::json;
use types::{
    ProofsCancelRequest,
    ProofsCancelResponse,
    ProofsFinalizeRequest,
    ProofsFinalizeResponse,
    ProofsGenerateRequest,
    ProofsGenerateResponse,
};

use crate::{
    error::WalletDaemonClientError,
    types::{
        AccountByNameRequest,
        AccountByNameResponse,
        AccountsCreateRequest,
        AccountsCreateResponse,
        AccountsGetBalancesRequest,
        AccountsGetBalancesResponse,
        AccountsInvokeRequest,
        AccountsInvokeResponse,
        AccountsListRequest,
        AccountsListResponse,
        KeysCreateRequest,
        KeysCreateResponse,
        KeysListRequest,
        KeysListResponse,
        KeysSetActiveRequest,
        KeysSetActiveResponse,
        TransactionGetRequest,
        TransactionGetResponse,
        TransactionGetResultRequest,
        TransactionGetResultResponse,
        TransactionSubmitRequest,
        TransactionSubmitResponse,
        TransactionWaitResultRequest,
        TransactionWaitResultResponse,
    },
};

#[derive(Debug, Clone)]
pub struct WalletDaemonClient {
    client: reqwest::Client,
    endpoint: Url,
    request_id: i64,
}

impl WalletDaemonClient {
    pub fn connect<T: IntoUrl>(endpoint: T) -> Result<Self, WalletDaemonClientError> {
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

    // pub async fn get_identity(&mut self) -> Result<GetIdentityResponse, WalletDaemonClientError> {
    //     self.send_request("identities.get", json!({})).await
    // }
    //
    // pub async fn get_active_templates(
    //     &mut self,
    //     request: GetTemplatesRequest,
    // ) -> Result<GetTemplatesResponse, WalletDaemonClientError> {
    //     self.send_request("templates.list", request).await
    // }
    //
    // pub async fn get_template(
    //     &mut self,
    //     request: GetTemplateRequest,
    // ) -> Result<GetTemplateResponse, WalletDaemonClientError> {
    //     self.send_request("templates.get", request).await
    // }

    pub async fn create_key(&mut self) -> Result<KeysCreateResponse, WalletDaemonClientError> {
        self.send_request("keys.create", &KeysCreateRequest {}).await
    }

    pub async fn set_active_key(&mut self, index: u64) -> Result<KeysSetActiveResponse, WalletDaemonClientError> {
        self.send_request("keys.set_active", &KeysSetActiveRequest { index })
            .await
    }

    pub async fn list_keys(&mut self) -> Result<KeysListResponse, WalletDaemonClientError> {
        self.send_request("keys.list", &KeysListRequest {}).await
    }

    pub async fn get_transaction<T: Borrow<TransactionGetRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionGetResponse, WalletDaemonClientError> {
        self.send_request("transactions.get", request.borrow()).await
    }

    pub async fn get_transaction_result<T: Borrow<TransactionGetResultRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionGetResultResponse, WalletDaemonClientError> {
        self.send_request("transactions.get_result", request.borrow()).await
    }

    pub async fn wait_transaction_result<T: Borrow<TransactionWaitResultRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionWaitResultResponse, WalletDaemonClientError> {
        self.send_request("transactions.wait_result", request.borrow()).await
    }

    pub async fn submit_transaction<T: Borrow<TransactionSubmitRequest>>(
        &mut self,
        request: T,
    ) -> Result<TransactionSubmitResponse, WalletDaemonClientError> {
        self.send_request("transactions.submit", request.borrow()).await
    }

    pub async fn create_account<T: Borrow<AccountsCreateRequest>>(
        &mut self,
        request: T,
    ) -> Result<AccountsCreateResponse, WalletDaemonClientError> {
        self.send_request("accounts.create", request.borrow()).await
    }

    pub async fn invoke_account_method<T: Borrow<AccountsInvokeRequest>>(
        &mut self,
        req: T,
    ) -> Result<AccountsInvokeResponse, WalletDaemonClientError> {
        self.send_request("accounts.invoke", req.borrow()).await
    }

    pub async fn get_account_balances<T: Borrow<AccountsGetBalancesRequest>>(
        &mut self,
        request: T,
    ) -> Result<AccountsGetBalancesResponse, WalletDaemonClientError> {
        self.send_request("accounts.get_balances", request.borrow()).await
    }

    pub async fn list_accounts(&mut self, limit: u64) -> Result<AccountsListResponse, WalletDaemonClientError> {
        self.send_request("accounts.list", &AccountsListRequest { limit }).await
    }

    pub async fn get_by_name(&mut self, name: String) -> Result<AccountByNameResponse, WalletDaemonClientError> {
        self.send_request("accounts.get_by_name", &AccountByNameRequest { name })
            .await
    }

    pub async fn create_transfer_proof<T: Borrow<ProofsGenerateRequest>>(
        &mut self,
        req: T,
    ) -> Result<ProofsGenerateResponse, WalletDaemonClientError> {
        self.send_request("confidential.create", req.borrow()).await
    }

    pub async fn cancel_transfer_proof<T: Borrow<ProofsCancelRequest>>(
        &mut self,
        req: T,
    ) -> Result<ProofsCancelResponse, WalletDaemonClientError> {
        self.send_request("confidential.cancel", req.borrow()).await
    }

    pub async fn finalize_transfer_proof<T: Borrow<ProofsFinalizeRequest>>(
        &mut self,
        req: T,
    ) -> Result<ProofsFinalizeResponse, WalletDaemonClientError> {
        self.send_request("confidential.finalize", req.borrow()).await
    }

    fn next_request_id(&mut self) -> i64 {
        self.request_id += 1;
        self.request_id
    }

    async fn send_request<T: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: &T,
    ) -> Result<R, WalletDaemonClientError> {
        let params = json::to_value(params).map_err(|e| WalletDaemonClientError::SerializeRequest {
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

        match serde_json::from_value(resp) {
            Ok(r) => Ok(r),
            Err(e) => Err(WalletDaemonClientError::DeserializeResponse {
                source: e,
                method: method.to_string(),
            }),
        }
    }
}

fn jsonrpc_result(val: json::Value) -> Result<json::Value, WalletDaemonClientError> {
    if let Some(err) = val.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
        return Err(WalletDaemonClientError::RequestFailedWithStatus {
            code,
            message: message.to_string(),
        });
    }

    let result = val
        .get("result")
        .ok_or_else(|| WalletDaemonClientError::InvalidResponse {
            message: "Missing result field".to_string(),
        })?;
    Ok(result.clone())
}
