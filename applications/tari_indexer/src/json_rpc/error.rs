//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JsonRpcResponse,
};

const LOG_TARGET: &str = "tari::indexer::json_rpc";

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
