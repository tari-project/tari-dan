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

use std::{net::SocketAddr, sync::Arc};

use axum::{
    http::{Response, Uri},
    response::IntoResponse,
    routing::get,
    Router,
};
use include_dir::{include_dir, Dir};
use log::{error, info};
use reqwest::StatusCode;

const LOG_TARGET: &str = "tari_validator_node::http_ui::server";

pub async fn run_http_ui_server(address: SocketAddr, json_rpc_address: Option<String>) -> Result<(), anyhow::Error> {
    let json_rpc_address = Arc::new(json_rpc_address);
    let router = Router::new()
        .route(
            "/json_rpc_address",
            get(|| async move {
                json_rpc_address
                    .as_ref()
                    .as_ref()
                    .map(|s| s.to_string())
                    .unwrap_or("NOT CONFIGURED".to_string())
            }),
        )
        .fallback(handler);

    info!(target: LOG_TARGET, "🌐 HTTP UI started at {}", address);
    axum::Server::bind(&address)
        .serve(router.into_make_service())
        .await
        .map_err(|err| {
            error!(target: LOG_TARGET, "HTTP UI encountered an error: {}", err);
            err
        })?;

    info!(target: LOG_TARGET, "Stopping HTTP UI");
    Ok(())
}

static PROJECT_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../tari_validator_node_web_ui/build");

async fn handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path();

    // If path starts with /, strip it.
    let path = path.strip_prefix('/').unwrap_or(path);

    // If the path is a file, return it. Otherwise use index.html (SPA)
    if let Some(lib_rs) = PROJECT_DIR
        .get_file(path)
        .or_else(|| PROJECT_DIR.get_file("index.html"))
    {
        if let Some(body) = lib_rs.contents_utf8() {
            return Response::builder()
                .status(StatusCode::OK)
                .body(body.to_owned())
                .unwrap();
        }
    }
    println!("Not found {:?}", path);
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body("".to_string())
        .unwrap()
}
