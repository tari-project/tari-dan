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
use tari_wallet_daemon_client::types::{
    WebRtcAcceptResponse,
    WebRtcCheckNotificationsResponse,
    WebRtcDenyResponse,
    WebRtcGetOldestResponse,
    WebRtcStartRequest,
    WebRtcStartResponse,
};

use super::HandlerContext;
use crate::webrtc::{make_request, webrtc_start_session, Request, Response, UserConfirmationRequest};

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
    if queue.is_empty() {
        Err(JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(
                JsonRpcErrorReason::InvalidParams,
                "There are no messages".to_string(),
                serde_json::Value::Null,
            ),
        ))
    } else {
        let UserConfirmationRequest {
            website_name,
            req: Request { id, method, params, .. },
            ..
        } = queue.front().unwrap();
        Ok(JsonRpcResponse::success(answer_id, WebRtcGetOldestResponse {
            id: *id,
            method: method.clone(),
            params: params.clone(),
            website_name: website_name.clone(),
        }))
    }
}

pub async fn handle_accept_request(
    context: Arc<HandlerContext>,
    value: JsonRpcExtractor,
    token: Option<String>,
    message_queue: Arc<Mutex<VecDeque<UserConfirmationRequest>>>,
    jrpc_address: SocketAddr,
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
    let req;
    let dc;
    {
        let mut queue = message_queue.lock().unwrap();
        let item = queue.pop_front().unwrap();
        req = item.req;
        dc = item.dc;
    }
    let result = match serde_json::from_str::<serde_json::Value>(&req.params) {
        Ok(params) => match make_request(jrpc_address, Some(req.token), req.method, params).await {
            Ok(response) => response.to_string(),
            Err(e) => e.to_string(),
        },
        Err(e) => e.to_string(),
    };
    let response = Response {
        payload: result,
        id: req.id,
    };
    let text = match serde_json::to_string(&response) {
        Ok(response) => response,
        Err(e) => e.to_string(),
    };
    if let Err(e) = dc.send_text(text).await {
        println!("Error {:?}", e);
    }
    Ok(JsonRpcResponse::success(answer_id, WebRtcAcceptResponse {}))
}

pub async fn handle_deny_request(
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
    let req;
    let dc;
    {
        let mut queue = message_queue.lock().unwrap();
        let item = queue.pop_front().unwrap();
        req = item.req;
        dc = item.dc;
    }
    let response = Response {
        payload: "Request denied".to_string(),
        id: req.id,
    };
    let text = match serde_json::to_string(&response) {
        Ok(response) => response,
        Err(e) => e.to_string(),
    };
    if let Err(e) = dc.send_text(text).await {
        println!("Error {:?}", e);
    }
    Ok(JsonRpcResponse::success(answer_id, WebRtcDenyResponse {}))
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
    let permissions_token = jwt
        .grant(webrtc_start_request.name.clone(), auth_token.0)
        .map_err(|e| {
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
            webrtc_start_request.name,
        )
        .await
        {
            error!(target: LOG_TARGET, "Error starting webrtc session: {}", err);
        }
    });
    Ok(JsonRpcResponse::success(answer_id, WebRtcStartResponse {}))
}
