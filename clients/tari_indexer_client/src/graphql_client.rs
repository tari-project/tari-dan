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

use anyhow::anyhow;
use reqwest::{header, header::HeaderMap, IntoUrl, Url};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct IndexerGraphQLClient {
    client: reqwest::Client,
    endpoint: Url,
    request_id: i64,
}

impl IndexerGraphQLClient {
    pub fn connect<T: IntoUrl>(endpoint: T) -> Result<Self, anyhow::Error> {
        let client = reqwest::Client::builder()
            .default_headers({
                let mut headers = HeaderMap::with_capacity(1);
                headers.insert(header::CONTENT_TYPE, "applications/json".parse().unwrap());
                headers
            })
            .build()?;

        Ok(Self {
            client,
            endpoint: endpoint.into_url()?,
            request_id: 0,
        })
    }

    pub fn next_request_id(&mut self) -> i64 {
        self.request_id += 1;
        self.request_id
    }

    pub async fn send_request<R: DeserializeOwned>(
        &mut self,
        query: &str,
        variables: Option<Value>,
        headers: Option<HeaderMap>,
    ) -> Result<R, anyhow::Error> {
        let body = json!({
            "query": query,
            "variables": variables
        });
        let mut req: reqwest::RequestBuilder = self.client.post(self.endpoint.clone());
        if let Some(headers) = headers {
            req = req.headers(headers);
        }
        let resp = req.body(serde_json::to_string(&body)?).send().await?;
        let val = resp.json::<Value>().await?;
        let data: Value = graphql_data(val)?;
        serde_json::from_value::<R>(data).map_err(|e| anyhow!("Failed to deserialize response: {}", e))
    }
}

fn graphql_data(val: Value) -> Result<Value, anyhow::Error> {
    if let Some(err) = val.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
        return Err(anyhow!("GraphQL error {}: {}", code, message));
    }

    let result = val.get("data").ok_or_else(|| anyhow!("Missing result field"))?;
    Ok(result.clone())
}
