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

use std::time::Duration;

use anyhow::anyhow;
use reqwest::{header, header::HeaderMap, IntoUrl, Url};
use serde::de::DeserializeOwned;
use serde_json as json;
use serde_json::json;

#[derive(Debug, Clone)]
pub struct ValidatorNodeClient {
    client: reqwest::Client,
    endpoint: Url,
    request_id: i64,
}

impl ValidatorNodeClient {
    pub fn connect<T: IntoUrl>(endpoint: T) -> Result<Self, anyhow::Error> {
        let client = reqwest::Client::builder()
            .default_headers({
                let mut headers = HeaderMap::with_capacity(1);
                headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
                headers
            })
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            client,
            endpoint: endpoint.into_url()?,
            request_id: 0,
        })
    }

    pub async fn register(&mut self) -> Result<u64, anyhow::Error> {
        let val: json::Value = self.send_request("register", json!({})).await?;
        let tx_id = val["transaction_id"]
            .as_u64()
            .ok_or_else(|| anyhow!("Wallet did not return tx_id"))?;
        Ok(tx_id)
    }

    fn next_request_id(&mut self) -> i64 {
        self.request_id += 1;
        self.request_id
    }

    async fn send_request<T: Into<json::Value>, R: DeserializeOwned>(
        &mut self,
        method: &str,
        body: T,
    ) -> Result<R, anyhow::Error> {
        let request_json = json!(
            {
                "jsonrpc": "2.0",
                "id": self.next_request_id(),
                "method": method,
                "params": body.into(),
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
        Ok(json::from_value(resp)?)
    }
}

fn jsonrpc_result(val: json::Value) -> Result<json::Value, anyhow::Error> {
    if let Some(err) = val.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
        return Err(anyhow!("JSON-RPC error {}: {}", code, message));
    }

    let result = val.get("result").ok_or_else(|| anyhow!("Missing result field"))?;
    Ok(result.clone())
}
