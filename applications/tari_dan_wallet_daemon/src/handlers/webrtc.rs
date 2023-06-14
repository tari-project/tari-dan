//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use log::*;
use tari_dan_wallet_sdk::apis::jwt::{JrpcPermission, JrpcPermissions};
use tari_shutdown::ShutdownSignal;
use tari_wallet_daemon_client::types::{WebRtcCheckNotificationsResponse, WebRtcStartRequest, WebRtcStartResponse};

use super::HandlerContext;
use crate::webrtc::{webrtc_start_session, UserConfirmationRequest};

const LOG_TARGET: &str = "tari::dan::wallet_daemon::json_rpc";

pub fn handle_check_notifications(
    context: Arc<HandlerContext>,
    value: JsonRpcExtractor,
    token: Option<String>,
    message_queue: Arc<Mutex<VecDeque<UserConfirmationRequest>>>,
) -> JrpcResult {
    let answer_id = value.get_answer_id();
    context
        .wallet_sdk()
        .jwt_api()
        .check_auth(token, &[JrpcPermission::Webrtc])
        .map_err(|e| {
            JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::ApplicationError(401),
                    format!("Not authorized: {e}"),
                    serde_json::Value::Null,
                ),
            )
        })?;
    let queue = message_queue.lock().unwrap();
    Ok(JsonRpcResponse::success(answer_id, WebRtcCheckNotificationsResponse {
        pending_requests_count: queue.len(),
    }))
}

pub fn handle_get_oldest_request(
    context: Arc<HandlerContext>,
    value: JsonRpcExtractor,
    token: Option<String>,
    message_queue: Arc<Mutex<VecDeque<UserConfirmationRequest>>>,
) -> JrpcResult {
}

pub fn handle_accept_request(
    context: Arc<HandlerContext>,
    value: JsonRpcExtractor,
    token: Option<String>,
    message_queue: Arc<Mutex<VecDeque<UserConfirmationRequest>>>,
) -> JrpcResult {
}

pub fn handle_deny_request(
    context: Arc<HandlerContext>,
    value: JsonRpcExtractor,
    token: Option<String>,
    message_queue: Arc<Mutex<VecDeque<UserConfirmationRequest>>>,
) -> JrpcResult {
}

pub fn handle_start(
    context: Arc<HandlerContext>,
    value: JsonRpcExtractor,
    token: Option<String>,
    shutdown_signal: Arc<ShutdownSignal>,
    addresses: (SocketAddr, SocketAddr),
    message_queue: Arc<Mutex<VecDeque<UserConfirmationRequest>>>,
) -> JrpcResult {
    let answer_id = value.get_answer_id();
    context
        .wallet_sdk()
        .jwt_api()
        .check_auth(token, &[JrpcPermission::Webrtc])
        .map_err(|e| {
            JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(
                    JsonRpcErrorReason::ApplicationError(401),
                    format!("Not authorized: {e}"),
                    serde_json::Value::Null,
                ),
            )
        })?;
    let webrtc_start_request = value.parse_params::<WebRtcStartRequest>()?;
    let shutdown_signal = (*shutdown_signal).clone();
    let permissions = serde_json::from_str::<JrpcPermissions>(&webrtc_start_request.permissions).map_err(|e| {
        JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(
                JsonRpcErrorReason::InternalError,
                e.to_string(),
                serde_json::Value::Null,
            ),
        )
    })?;
    let jwt = context.wallet_sdk().jwt_api();
    let auth_token = jwt.generate_auth_token(permissions, None).map_err(|e| {
        JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(
                JsonRpcErrorReason::InternalError,
                e.to_string(),
                serde_json::Value::Null,
            ),
        )
    })?;
    let permissions_token = jwt.grant(webrtc_start_request.name, auth_token.0).map_err(|e| {
        JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(
                JsonRpcErrorReason::InternalError,
                e.to_string(),
                serde_json::Value::Null,
            ),
        )
    })?;
    tokio::spawn(async move {
        let (preferred_address, signaling_server_address) = addresses;
        if let Err(err) = webrtc_start_session(
            webrtc_start_request.signaling_server_token,
            permissions_token,
            preferred_address,
            signaling_server_address,
            shutdown_signal,
            message_queue,
        )
        .await
        {
            error!(target: LOG_TARGET, "Error starting webrtc session: {}", err);
        }
    });
    Ok(JsonRpcResponse::success(answer_id, WebRtcStartResponse {}))
}
