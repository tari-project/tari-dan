//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{str::FromStr, sync::Arc};

use axum::{
    handler::HandlerWithoutStateExt,
    http::{HeaderValue, Response, Uri},
    response::IntoResponse,
    routing::post,
    Extension,
    Router,
};
use axum_jrpc::{
    error::{JsonRpcError, JsonRpcErrorReason},
    JrpcResult,
    JsonRpcAnswer,
    JsonRpcExtractor,
    JsonRpcResponse,
};
use include_dir::{include_dir, Dir};
use log::*;
use reqwest::{header, StatusCode};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::json;
use tower_http::{cors::CorsLayer, services::ServeDir};

use crate::webserver::{context::HandlerContext, error::HandlerError, handler::JrpcHandler, rpc, templates};

const LOG_TARGET: &str = "tari::dan::swarm::webserver";

pub async fn run(context: HandlerContext) -> anyhow::Result<()> {
    let bind_address = context.config().webserver.bind_address;

    async fn not_found() -> (StatusCode, &'static str) {
        (StatusCode::NOT_FOUND, "Resource not found")
    }

    let serve_templates =
        ServeDir::new(context.config().base_dir.join("templates")).not_found_service(not_found.into_service());

    let serve_misc = ServeDir::new(context.config().base_dir.join("misc")).not_found_service(not_found.into_service());

    let router = Router::new()
        .route("/upload_template", post(templates::upload))
        .route("/json_rpc", post(json_rpc_handler))
        .nest_service("/templates", serve_templates)
        .nest_service("/misc", serve_misc)
        .fallback(handler)
        .layer(Extension(Arc::new(context)))
        .layer(CorsLayer::permissive());

    let server = axum::Server::try_bind(&bind_address).or_else(|_| {
        error!(
            target: LOG_TARGET,
            "🕸️ Failed to bind on preferred address {}. Trying OS-assigned", bind_address
        );
        axum::Server::try_bind(&"127.0.0.1:0".parse().unwrap())
    })?;

    let server = server.serve(router.into_make_service());
    info!(target: LOG_TARGET, "🕸️ Webserver listening on {}", server.local_addr());
    server.await?;

    Ok(())
}

static WEB_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/webui/dist");

async fn handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path();

    // If path starts with /, strip it.
    let path = path.strip_prefix('/').unwrap_or(path);

    // If the path is a file, return it. Otherwise, use index.html (SPA)
    if let Some(body) = WEB_DIR
        .get_file(path)
        .or_else(|| WEB_DIR.get_file("index.html"))
        .and_then(|file| file.contents_utf8())
    {
        let mime_type = mime_guess::from_path(path).first_or_else(|| mime_guess::Mime::from_str("text/html").unwrap());
        return Response::builder()
            .header(header::CONTENT_TYPE, HeaderValue::from_str(mime_type.as_ref()).unwrap())
            .status(StatusCode::OK)
            .body(body.to_owned())
            .unwrap();
    }
    log::warn!(target: LOG_TARGET, "Not found {:?}", path);
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body("".to_string())
        .unwrap()
}

async fn json_rpc_handler(Extension(context): Extension<Arc<HandlerContext>>, value: JsonRpcExtractor) -> JrpcResult {
    info!(target: LOG_TARGET, "🌐 JSON-RPC request: {}", value.method);
    debug!(target: LOG_TARGET, "🌐 JSON-RPC request: {:?}", value);
    match value.method.as_str() {
        "ping" => Ok(JsonRpcResponse::success(value.get_answer_id(), "pong")),
        "vns" => call_handler(context, value, rpc::validator_nodes::list).await,
        "dan_wallets" => call_handler(context, value, rpc::dan_wallets::list).await,
        "indexers" => call_handler(context, value, rpc::indexers::list).await,
        "get_logs" => call_handler(context, value, rpc::logs::list_log_files).await,
        "get_stdout" => call_handler(context, value, rpc::logs::list_stdout_files).await,
        "get_file" => call_handler(context, value, rpc::logs::get_log_file).await,
        "mine" => call_handler(context, value, rpc::miners::mine).await,
        "is_mining" => call_handler(context, value, rpc::miners::is_mining).await,
        "start_mining" => call_handler(context, value, rpc::miners::start_mining).await,
        "stop_mining" => call_handler(context, value, rpc::miners::stop_mining).await,
        "add_base_node" | "add_minotari_node" => call_handler(context, value, rpc::minotari_nodes::create).await,
        "add_base_wallet" | "add_minotari_wallet" => call_handler(context, value, rpc::minotari_wallets::create).await,
        "add_asset_wallet" | "add_wallet_daemon" => call_handler(context, value, rpc::dan_wallets::create).await,
        "add_indexer" => call_handler(context, value, rpc::indexers::create).await,
        "add_validator_node" => call_handler(context, value, rpc::validator_nodes::create).await,
        "start" => call_handler(context, value, rpc::instances::start).await,
        "start_all" => call_handler(context, value, rpc::instances::start_all).await,
        "stop" => call_handler(context, value, rpc::instances::stop).await,
        "stop_all" => call_handler(context, value, rpc::instances::stop_all).await,
        "list_instances" => call_handler(context, value, rpc::instances::list).await,
        "delete_data" => call_handler(context, value, rpc::instances::delete_data).await,
        "burn_funds" => call_handler(context, value, rpc::minotari_wallets::burn_funds).await,
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
    H: for<'a> JrpcHandler<'a, TReq, Response = TResp>,
{
    let answer_id = value.get_answer_id();
    let params = value.parse_params().map_err(|e| {
        match &e.result {
            JsonRpcAnswer::Result(_) => {
                unreachable!("parse_params() error should not return a result")
            },
            JsonRpcAnswer::Error(e) => {
                warn!(target: LOG_TARGET, "🌐 JSON-RPC params error: {}", e);
            },
        }
        e
    })?;
    let resp = handler
        .handle(&context, params)
        .await
        .map_err(|e| resolve_handler_error(answer_id, &e))?;
    Ok(JsonRpcResponse::success(answer_id, resp))
}

fn resolve_handler_error(answer_id: i64, e: &HandlerError) -> JsonRpcResponse {
    match e {
        HandlerError::Anyhow(e) => resolve_any_error(answer_id, e),
        // HandlerError::NotFound => JsonRpcResponse::error(
        //     answer_id,
        //     JsonRpcError::new(JsonRpcErrorReason::ApplicationError(404), e.to_string(), json!({})),
        // ),
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
