//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{net::SocketAddr, sync::Arc};

use axum_extra::headers::authorization::Bearer;
use axum_jrpc::{
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
    error::{JsonRpcError, JsonRpcErrorReason},
};
use log::*;
use tari_ootle_walletd_client::{
    permissions::{Permission, Permissions},
    types::{WebRtcStartRequest, WebRtcStartResponse},
};

use super::HandlerContext;
use crate::{server::ApplicationErrorCode, webrtc::webrtc_start_session};

const LOG_TARGET: &str = "tari::ootle::wallet_daemon::json_rpc";

pub fn handle_start(
    context: Arc<HandlerContext>,
    value: JsonRpcExtractor,
    token: Option<&Bearer>,
    addresses: (SocketAddr, SocketAddr),
) -> JrpcResult {
    let answer_id = value.get_answer_id();
    if !context.config().enable_webrtc {
        return Err(JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(
                JsonRpcErrorReason::ApplicationError(ApplicationErrorCode::InvalidRequest as i32),
                "WebRTC connections are disabled on this wallet daemon. Start it with --enable-webrtc (or set \
                 enable_webrtc = true) to let apps connect."
                    .to_string(),
                serde_json::Value::Null,
            ),
        ));
    }
    let granted = context.authorize(token, &[Permission::Webrtc]).map_err(|e| {
        JsonRpcResponse::error(
            answer_id.clone(),
            JsonRpcError::new(
                JsonRpcErrorReason::ApplicationError(ApplicationErrorCode::Unauthorized as i32),
                format!("Not authorized: {e}"),
                serde_json::Value::Null,
            ),
        )
    })?;
    let webrtc_start_request = value.parse_params::<WebRtcStartRequest>()?;
    let requested_permissions =
        serde_json::from_value::<Permissions>(webrtc_start_request.permissions).map_err(|e| {
            JsonRpcResponse::error(
                answer_id.clone(),
                JsonRpcError::new(
                    JsonRpcErrorReason::InvalidRequest,
                    e.to_string(),
                    serde_json::Value::Null,
                ),
            )
        })?;
    // The session token must never carry more than the caller's own grant, so drop anything
    // requested that the caller isn't already authorised for instead of trusting it verbatim.
    let permissions: Permissions = requested_permissions
        .into_vec()
        .into_iter()
        .filter(|p| granted.satisfies(p))
        .collect();
    let jwt = context.jwt_api();
    let mut claim = jwt.generate_auth_claims(permissions).map_err(|e| {
        JsonRpcResponse::error(
            answer_id.clone(),
            JsonRpcError::new(
                JsonRpcErrorReason::InternalError,
                e.to_string(),
                serde_json::Value::Null,
            ),
        )
    })?;
    claim.delegated = true;
    claim.delegation_id = Some(rand::random());
    let permissions_token = jwt.grant(&claim).map_err(|e| {
        JsonRpcResponse::error(
            answer_id.clone(),
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
            context.shutdown_signal().clone(),
        )
        .await
        {
            error!(target: LOG_TARGET, "Error starting webrtc session: {}", err);
        }
    });
    Ok(JsonRpcResponse::success(answer_id, WebRtcStartResponse {}))
}

#[cfg(test)]
mod tests {
    use axum_jrpc::Id;

    use super::*;
    use crate::handlers::test_support::TestDaemon;

    #[tokio::test]
    async fn webrtc_start_is_refused_unless_enabled() {
        let daemon = TestDaemon::start().await;
        assert!(!daemon.context.config().enable_webrtc);
        let api_key = daemon.api_key("dapp-bridge", "admin");

        let request = JsonRpcExtractor {
            parsed: serde_json::json!({ "signaling_server_token": "token", "permissions": ["Admin"] }),
            method: "webrtc.start".to_string(),
            id: Id::Num(1),
        };
        let addresses = ("127.0.0.1:1".parse().unwrap(), "127.0.0.1:2".parse().unwrap());
        let response = handle_start(Arc::new(daemon.context.clone()), request, Some(&api_key), addresses).unwrap_err();

        let body = serde_json::to_string(&response).unwrap();
        assert!(body.contains("WebRTC connections are disabled"), "{body}");
    }
}
