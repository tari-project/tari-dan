//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{net::SocketAddr, str::FromStr, sync::Arc};

use axum::{
    http::{HeaderValue, Response, Uri},
    response::IntoResponse,
    routing::post,
    Extension,
    Router,
};
use axum_jrpc::{JrpcResult, JsonRpcExtractor, JsonRpcResponse};
use include_dir::{include_dir, Dir};
use log::*;
use reqwest::{header, StatusCode};

use crate::webserver::context::HandlerContext;

const LOG_TARGET: &str = "tari::dan::swarm::webserver";

pub async fn run(context: HandlerContext) -> anyhow::Result<()> {
    let bind_address = context.config().bind_address.clone();
    let bind_address = SocketAddr::from_str(&bind_address)?;
    let router = Router::new()
        .layer(Extension(Arc::new(context)))
        .route("/json_rpc", post(json_rpc_handler))
        .fallback(handler);

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

static WEB_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../webui/dist");

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

async fn json_rpc_handler(Extension(_context): Extension<Arc<HandlerContext>>, value: JsonRpcExtractor) -> JrpcResult {
    info!(target: LOG_TARGET, "🌐 JSON-RPC request: {}", value.method);
    debug!(target: LOG_TARGET, "🌐 JSON-RPC request: {:?}", value);
    let method_parts = value
        .method
        .as_str()
        .split_once('.')
        .map(|(l, r)| (l, Some(r)))
        .unwrap_or((value.method.as_str(), None));
    match method_parts {
        ("ping", None) => Ok(JsonRpcResponse::success(value.get_answer_id(), "pong")),
        _ => Ok(value.method_not_found(&value.method)),
    }
}

// TODO: implement handlers
// async fn call_handler<H, TReq, TResp>(
//     context: Arc<HandlerContext>,
//     value: JsonRpcExtractor,
//     mut handler: H,
// ) -> JrpcResult
// where
//     TReq: DeserializeOwned,
//     TResp: Serialize,
//     H: for<'a> JrpcHandler<'a, TReq, Response = TResp>,
// {
//     let answer_id = value.get_answer_id();
//     let params = value.parse_params().map_err(|e| {
//         match &e.result {
//             JsonRpcAnswer::Result(_) => {
//                 unreachable!("parse_params() error should not return a result")
//             },
//             JsonRpcAnswer::Error(e) => {
//                 warn!(target: LOG_TARGET, "🌐 JSON-RPC params error: {}", e);
//             },
//         }
//         e
//     })?;
//     let resp = handler
//         .handle(&context, params)
//         .await
//         .map_err(|e| resolve_handler_error(answer_id, &e))?;
//     Ok(JsonRpcResponse::success(answer_id, resp))
// }
//
// fn resolve_handler_error(answer_id: i64, e: &HandlerError) -> JsonRpcResponse {
//     match e {
//         HandlerError::Anyhow(e) => resolve_any_error(answer_id, e),
//         HandlerError::NotFound => JsonRpcResponse::error(
//             answer_id,
//             JsonRpcError::new(JsonRpcErrorReason::ApplicationError(404), e.to_string(), json!({})),
//         ),
//     }
// }
//
// fn resolve_any_error(answer_id: i64, e: &anyhow::Error) -> JsonRpcResponse {
//     warn!(target: LOG_TARGET, "🌐 JSON-RPC error: {}", e);
//     if let Some(handler_err) = e.downcast_ref::<HandlerError>() {
//         return resolve_handler_error(answer_id, handler_err);
//     }
//
//     JsonRpcResponse::error(
//         answer_id,
//         JsonRpcError::new(JsonRpcErrorReason::ApplicationError(500), e.to_string(), json!({})),
//     )
// }
