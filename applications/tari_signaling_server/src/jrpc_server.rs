//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fs,
    net::SocketAddr,
    path::PathBuf,
    process,
    sync::{Arc, Mutex, MutexGuard},
};

use axum::{
    extract::Extension,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
    routing::post,
    Router,
};
use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use log::*;
use serde_json::json;
use tari_dan_wallet_sdk::apis::jwt::JrpcPermissions;
use tari_shutdown::ShutdownSignal;
use tower_http::cors::CorsLayer;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;

use crate::data::Data;

// use super::handlers::HandlerContext;
// use crate::handlers::{accounts, confidential, error::HandlerError, keys, rpc, transaction, Handler};

const LOG_TARGET: &str = "tari::signaling_server::json_rpc";

// We need to extract the token, because the first call is without any token. So we don't have to have two handlers.
async fn extract_token<B>(mut request: Request<B>, next: Next<B>) -> Result<Response, StatusCode> {
    let mut token_ext = None;
    if let Some(token) = request.headers().get("authorization") {
        if let Ok(token) = token.to_str() {
            if let Some(token) = token.strip_prefix("Bearer ") {
                token_ext = Some(token.to_string());
            }
        }
    }
    request.extensions_mut().insert::<Option<String>>(token_ext);
    let response = next.run(request).await;
    Ok(response)
}

pub async fn listen(
    base_dir: PathBuf,
    preferred_address: SocketAddr,
    data: Data,
    shutdown_signal: ShutdownSignal,
) -> Result<(), anyhow::Error> {
    let router = Router::new()
        .route("/", post(handler))
        .route("/json_rpc", post(handler))
        .layer(Extension(Arc::new(Mutex::new(data))))
        .layer(CorsLayer::permissive())
        .layer(axum::middleware::from_fn(extract_token));

    let server = axum::Server::try_bind(&preferred_address)?;
    let server = server.serve(router.into_make_service());
    info!(target: LOG_TARGET, "🌐 JSON-RPC listening on {}", server.local_addr());
    let server = server.with_graceful_shutdown(shutdown_signal);
    fs::write(base_dir.join("pid"), process::id().to_string())?;
    server.await?;

    info!(target: LOG_TARGET, "💤 Stopping JSON-RPC");
    Ok(())
}

fn add_offer(
    id: u64,
    value: &JsonRpcExtractor,
    mut data: MutexGuard<Data>,
) -> std::result::Result<std::string::String, anyhow::Error> {
    info!(
        target: LOG_TARGET,
        "Adding offer to id {id} : {}",
        value.parsed.as_str().unwrap()
    );
    data.add_offer(id, value.parsed.as_str().unwrap().to_string());
    Ok(serde_json::to_string("").unwrap())
}

fn add_offer_ice_candidate(
    id: u64,
    value: &JsonRpcExtractor,
    mut data: MutexGuard<Data>,
) -> std::result::Result<std::string::String, anyhow::Error> {
    info!(
        target: LOG_TARGET,
        "Adding offer ice candidate to id {id} : {}", value.parsed
    );
    match serde_json::from_value::<RTCIceCandidateInit>(value.parsed.clone()) {
        Ok(ice_candidate) => {
            data.add_offer_ice_candidate(id, ice_candidate);
            Ok(serde_json::to_string("").unwrap())
        },
        Err(e) => Err(anyhow::anyhow!(e)),
    }
}

fn add_answer(
    id: u64,
    value: &JsonRpcExtractor,
    mut data: MutexGuard<Data>,
) -> std::result::Result<std::string::String, anyhow::Error> {
    info!(target: LOG_TARGET, "Adding answer to id {id} : {}", value.parsed);
    data.add_answer(id, value.parsed.to_string());
    Ok(serde_json::to_string("").unwrap())
}

fn add_answer_ice_candidate(
    id: u64,
    value: &JsonRpcExtractor,
    mut data: MutexGuard<Data>,
) -> std::result::Result<std::string::String, anyhow::Error> {
    info!(
        target: LOG_TARGET,
        "Adding answer ice candidate to id {id} : {}", value.parsed
    );
    match serde_json::from_value::<RTCIceCandidateInit>(value.parsed.clone()) {
        Ok(ice_candidate) => {
            data.add_answer_ice_candidate(id, ice_candidate);
            Ok(serde_json::to_string("").unwrap())
        },
        Err(e) => Err(anyhow::anyhow!(e)),
    }
}

fn get_offer(id: u64, data: MutexGuard<Data>) -> std::result::Result<std::string::String, anyhow::Error> {
    info!(target: LOG_TARGET, "Getting offer for id {id}");
    println!("Offer {}", data.get_offer(id).map(|res| res.clone()).unwrap());
    data.get_offer(id).map(|res| res.clone())
}

fn get_answer(id: u64, data: MutexGuard<Data>) -> std::result::Result<std::string::String, anyhow::Error> {
    info!(target: LOG_TARGET, "Getting answer for id {id}");
    data.get_answer(id).map(|res| res.clone())
}

fn get_offer_ice_candidates(
    id: u64,
    data: MutexGuard<Data>,
) -> std::result::Result<std::string::String, anyhow::Error> {
    info!(target: LOG_TARGET, "Getting offer ice candidate for id {id}");
    Ok(serde_json::to_string(data.get_offer_ice_candidates(id).unwrap()).unwrap())
}
fn get_answer_ice_candidates(
    id: u64,
    data: MutexGuard<Data>,
) -> std::result::Result<std::string::String, anyhow::Error> {
    info!(target: LOG_TARGET, "Getting answer ice candidate for id {id}");
    Ok(serde_json::to_string(data.get_answer_ice_candidates(id).unwrap()).unwrap())
}

async fn handler(
    Extension(data): Extension<Arc<Mutex<Data>>>,
    Extension(token): Extension<Option<String>>,
    value: JsonRpcExtractor,
) -> JrpcResult {
    let answer_id = value.get_answer_id();
    let mut data = data.lock().unwrap();
    let result;
    if let Some(token) = token {
        let id = match data.check_jwt(token) {
            Ok(id) => id,
            Err(e) => {
                return Ok(JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(JsonRpcErrorReason::ApplicationError(401), format!("{}", e), json!({})),
                ));
            },
        };
        result = match value.method() {
            "add.offer" => add_offer(id, &value, data),
            "add.offer_ice_candidate" => add_offer_ice_candidate(id, &value, data),
            "add.answer" => add_answer(id, &value, data),
            "add.answer_ice_candidate" => add_answer_ice_candidate(id, &value, data),
            "get.offer" => get_offer(id, data),
            "get.answer" => get_answer(id, data),
            "get.offer_ice_candidates" => get_offer_ice_candidates(id, data),
            "get.answer_ice_candidates" => get_answer_ice_candidates(id, data),
            _ => {
                error!(target: LOG_TARGET, "Method not found {}", value.method);
                return Ok(JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(JsonRpcErrorReason::MethodNotFound, value.method, json!({})),
                ));
            },
        }
    } else {
        result = match value.method() {
            "auth.login" => {
                info!(target: LOG_TARGET, "Generating new JWT token");
                let permissions = serde_json::from_value::<JrpcPermissions>(value.parsed).map_err(|e| {
                    JsonRpcResponse::error(
                        answer_id,
                        JsonRpcError::new(
                            JsonRpcErrorReason::InternalError,
                            e.to_string(),
                            serde_json::Value::Null,
                        ),
                    )
                })?;
                data.generate_jwt(permissions)
            },
            _ => {
                error!(
                    target: LOG_TARGET,
                    "Without bearer token there is only one method available \"auth.login\""
                );
                return Ok(JsonRpcResponse::error(
                    answer_id,
                    JsonRpcError::new(JsonRpcErrorReason::ApplicationError(401), "".to_string(), json!({})),
                ));
            },
        };
    }

    match result {
        Ok(payload) => Ok(JsonRpcResponse::success(answer_id, payload)),
        Err(e) => {
            error!(target: LOG_TARGET, "Error {:?}", e);
            Ok(JsonRpcResponse::error(
                answer_id,
                JsonRpcError::new(JsonRpcErrorReason::ApplicationError(500), value.method, json!({})),
            ))
        },
    }
}
