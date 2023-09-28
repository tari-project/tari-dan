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

const LOG_TARGET: &str = "tari::validator_node::json_rpc::handlers";

use std::fmt::Display;

use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JsonRpcResponse,
};

// pub fn invalid_argument<T: Display>(answer_id: i64) -> impl Fn(T) -> JsonRpcResponse {
//     move |err| {
//         log::error!(target: LOG_TARGET, "🚨 Invalid argument: {}", err);
//         JsonRpcResponse::error(
//             answer_id,
//             JsonRpcError::new(
//                 JsonRpcErrorReason::InvalidParams,
//                 format!("Invalid argument: {}", err),
//                 serde_json::Value::Null,
//             ),
//         )
//     }
// }

pub fn internal_error<T: Display>(answer_id: i64) -> impl Fn(T) -> JsonRpcResponse {
    move |err| {
        let msg = if cfg!(debug_assertions) || option_env!("CI").is_some() {
            err.to_string()
        } else {
            log::error!(target: LOG_TARGET, "🚨 Internal error: {}", err);
            "Something went wrong".to_string()
        };
        JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(JsonRpcErrorReason::InternalError, msg, serde_json::Value::Null),
        )
    }
}

pub fn not_found<T: Into<String>>(answer_id: i64, details: T) -> JsonRpcResponse {
    JsonRpcResponse::error(
        answer_id,
        JsonRpcError::new(
            JsonRpcErrorReason::ApplicationError(404),
            details.into(),
            serde_json::Value::Null,
        ),
    )
}
