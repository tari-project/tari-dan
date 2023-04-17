//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{net::SocketAddr, sync::Arc};

use axum::{extract::Extension, routing::post, Router};
use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcAnswer,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::json;
use tari_shutdown::ShutdownSignal;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use super::handlers::HandlerContext;
use crate::{
    handlers::{accounts, confidential, error::HandlerError, keys, rpc, transaction, Handler},
    webrtc::webrtc_start_session,
};

const LOG_TARGET: &str = "tari::dan_wallet_daemon::json_rpc";

pub async fn listen(
    preferred_address: SocketAddr,
    signaling_server_address: SocketAddr,
    context: HandlerContext,
    shutdown_signal: ShutdownSignal,
) -> Result<(), anyhow::Error> {
    let router = Router::new()
        .route("/", post(handler))
        .route("/json_rpc", post(handler))
        // TODO: Get these traces to work
        .layer(TraceLayer::new_for_http())
        .layer(Extension(Arc::new(context)))
        .layer(Extension(Arc::new((preferred_address,signaling_server_address))))
        .layer(Extension(Arc::new(shutdown_signal.clone())))
        .layer(CorsLayer::permissive());

    let server = axum::Server::try_bind(&preferred_address)?;
    let server = server.serve(router.into_make_service());
    info!(target: LOG_TARGET, "🌐 JSON-RPC listening on {}", server.local_addr());
    let server = server.with_graceful_shutdown(shutdown_signal);
    server.await?;

    info!(target: LOG_TARGET, "💤 Stopping JSON-RPC");
    Ok(())
}

async fn handler(
    Extension(context): Extension<Arc<HandlerContext>>,
    Extension(addresses): Extension<Arc<(SocketAddr, SocketAddr)>>,
    Extension(shutdown_signal): Extension<Arc<ShutdownSignal>>,
    value: JsonRpcExtractor,
) -> JrpcResult {
    info!(target: LOG_TARGET, "🌐 JSON-RPC request: {}", value.method);

    match value.method.as_str().split_once('.') {
        Some(("rpc", "discover")) => call_handler(context, value, rpc::handle_discover).await,
        Some(("keys", method)) => match method {
            "create" => call_handler(context, value, keys::handle_create).await,
            "list" => call_handler(context, value, keys::handle_list).await,
            "set_active" => call_handler(context, value, keys::handle_set_active).await,
            _ => Ok(value.method_not_found(&value.method)),
        },
        Some(("transactions", method)) => match method {
            "submit" => call_handler(context, value, transaction::handle_submit).await,
            "get" => call_handler(context, value, transaction::handle_get).await,
            "get_result" => call_handler(context, value, transaction::handle_get_result).await,
            "wait_result" => call_handler(context, value, transaction::handle_wait_result).await,
            _ => Ok(value.method_not_found(&value.method)),
        },
        Some(("accounts", method)) => match method {
            "reveal_funds" => call_handler(context, value, accounts::handle_reveal_funds).await,
            "claim_burn" => call_handler(context, value, accounts::handle_claim_burn).await,
            "create" => call_handler(context, value, accounts::handle_create).await,
            "list" => call_handler(context, value, accounts::handle_list).await,
            "get_balances" => call_handler(context, value, accounts::handle_get_balances).await,
            "invoke" => call_handler(context, value, accounts::handle_invoke).await,
            "get_by_name" => call_handler(context, value, accounts::handle_get_by_name).await,
            "confidential_transfer" => call_handler(context, value, accounts::handle_confidential_transfer).await,
            _ => Ok(value.method_not_found(&value.method)),
        },
        Some(("confidential", method)) => match method {
            "create_transfer_proof" => call_handler(context, value, confidential::handle_create_transfer_proof).await,
            "finalize" => call_handler(context, value, confidential::handle_finalize_transfer).await,
            "cancel" => call_handler(context, value, confidential::handle_cancel_transfer).await,
            "create_output_proof" => call_handler(context, value, confidential::handle_create_output_proof).await,
            _ => Ok(value.method_not_found(&value.method)),
        },
        Some(("webrtc", "start")) => {
            let answer_id = value.get_answer_id();
            let token = value.parse_params::<String>()?;
            let shutdown_signal = (*shutdown_signal).clone();
            let (preferred_address, signaling_server_address) = *addresses.clone();
            tokio::spawn(async move {
                webrtc_start_session(token, preferred_address, signaling_server_address, shutdown_signal)
                    .await
                    .unwrap();
            });
            Ok(JsonRpcResponse::success(answer_id, "started"))
        },
        _ => Ok(value.method_not_found(&value.method)),
    }
}

async fn call_handler<H, TReq, TResp>(
    context: Arc<HandlerContext>,
    value: JsonRpcExtractor,
    mut handler: H,
) -> JrpcResult
where
    TReq: DeserializeOwned,
    TResp: Serialize,
    H: for<'a> Handler<'a, TReq, Response = TResp>,
{
    let answer_id = value.get_answer_id();
    let resp = handler
        .handle(
            &context,
            value.parse_params().map_err(|e| {
                match &e.result {
                    JsonRpcAnswer::Result(_) => {
                        unreachable!("parse_params should not return a result")
                    },
                    JsonRpcAnswer::Error(e) => {
                        warn!(target: LOG_TARGET, "🌐 JSON-RPC params error: {}", e);
                    },
                }
                e
            })?,
        )
        .await
        .map_err(|e| resolve_handler_error(answer_id, &e))?;
    Ok(JsonRpcResponse::success(answer_id, resp))
}

fn resolve_handler_error(answer_id: i64, e: &HandlerError) -> JsonRpcResponse {
    dbg!("here2");
    match e {
        HandlerError::Anyhow(e) => resolve_any_error(answer_id, e),
        HandlerError::NotFound => JsonRpcResponse::error(
            answer_id,
            JsonRpcError::new(JsonRpcErrorReason::ApplicationError(404), e.to_string(), json!({})),
        ),
    }
}

fn resolve_any_error(answer_id: i64, e: &anyhow::Error) -> JsonRpcResponse {
    warn!(target: LOG_TARGET, "🌐 JSON-RPC error: {}", e);
    if let Some(handler_err) = e.downcast_ref::<HandlerError>() {
        return resolve_handler_error(answer_id, handler_err);
    }
    JsonRpcResponse::error(
        answer_id,
        JsonRpcError::new(JsonRpcErrorReason::ApplicationError(500), e.to_string(), json!({})),
    )
}
