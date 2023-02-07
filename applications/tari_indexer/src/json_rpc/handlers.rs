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

use axum_jrpc::{JrpcResult, JsonRpcExtractor, JsonRpcResponse};
use serde::Serialize;
use tari_engine_types::substate::SubstateAddress;

const _LOG_TARGET: &str = "tari::indexer::json_rpc::handlers";

pub struct JsonRpcHandlers {
    addresses: Vec<SubstateAddress>,
}

impl JsonRpcHandlers {
    pub fn new(addresses: Vec<SubstateAddress>) -> Self {
        Self { addresses }
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
}

#[derive(Serialize, Debug)]
struct GetStatusResponse {
    addresses: Vec<String>,
}
