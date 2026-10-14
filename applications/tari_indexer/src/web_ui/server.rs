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
    Router,
    http::{Response, StatusCode, Uri, header::CONTENT_TYPE},
    response::IntoResponse,
    routing::get,
};
use include_dir::{Dir, include_dir};
use log::*;
use tari_ootle_app_utilities::tcp::try_bind_with_fallback;
use url::Url;

const LOG_TARGET: &str = "tari::indexer::web_ui::server";

pub async fn serve(address: SocketAddr, api_address: Url) -> Result<(), anyhow::Error> {
    let api_address = Arc::new(api_address);

    let router = Router::new()
        .route("/rest_api_address", get(|| async move { api_address.to_string() }))
        .fallback(handler);

    info!(target: LOG_TARGET, "🕸️ Web UI started at http://{}", address);
    let listener = try_bind_with_fallback(address).await?;
    let server = axum::serve(listener, router);
    info!(target: LOG_TARGET, "🕸️ Web UI listening on {}", server.local_addr()?);
    server.await?;

    info!(target: LOG_TARGET, "Stopping Web UI");
    Ok(())
}

static PROJECT_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/web_ui/dist");

async fn handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path();

    // If path starts with /, strip it.
    let path = path.strip_prefix('/').unwrap_or(path);

    // If the path is a file, return it. Otherwise use index.html (SPA)
    let (file, path) = match PROJECT_DIR.get_file(path) {
        Some(file) => (file, path),
        None => (PROJECT_DIR.get_file("index.html").unwrap(), "index.html"),
    };
    if let Some(body) = file.contents_utf8() {
        let content_type = mime_guess::from_path(path).first_raw().unwrap_or("text/html");
        return Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, content_type)
            .body(body.to_owned())
            .unwrap();
    }
    log::warn!(target: LOG_TARGET, "Path not found: {}", path);
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body("".to_string())
        .unwrap()
}
