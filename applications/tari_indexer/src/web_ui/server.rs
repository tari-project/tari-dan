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
    body::Body,
    http::{
        Response,
        StatusCode,
        Uri,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
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
    serve_file(&PROJECT_DIR, uri.path())
}

/// Vite names everything under `assets/` by its content hash, so a cached copy is valid for as long as its name is
/// requested. Every other file, index.html above all, names those assets and must be revalidated.
const HASHED_ASSET_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";
const REVALIDATE_CACHE_CONTROL: &str = "no-cache";

/// The file at `path` in `dir`, or `dir`'s index.html for any other path so the SPA router can resolve it.
fn serve_file(dir: &'static Dir<'static>, path: &str) -> Response<Body> {
    let path = path.strip_prefix('/').unwrap_or(path);
    let (file, path) = match dir.get_file(path) {
        Some(file) => (file, path),
        None => (dir.get_file("index.html").unwrap(), "index.html"),
    };
    let content_type = mime_guess::from_path(path).first_raw().unwrap_or("text/html");
    let cache_control = if path.starts_with("assets/") {
        HASHED_ASSET_CACHE_CONTROL
    } else {
        REVALIDATE_CACHE_CONTROL
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, content_type)
        .header(CACHE_CONTROL, cache_control)
        .body(Body::from(file.contents()))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use include_dir::{DirEntry, File};

    use super::*;

    const WASM: &[u8] = b"\0asm\x01\0\0\0\xff\xfe";

    static DIR: Dir<'static> = Dir::new("", &[
        DirEntry::File(File::new("index.html", b"<html></html>")),
        DirEntry::Dir(Dir::new("assets", &[DirEntry::File(File::new(
            "assets/decoder.wasm",
            WASM,
        ))])),
    ]);

    async fn body(response: Response<Body>) -> Vec<u8> {
        to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()
    }

    #[tokio::test]
    async fn a_binary_asset_is_served_as_is() {
        let response = serve_file(&DIR, "/assets/decoder.wasm");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "application/wasm");
        assert_eq!(response.headers()[CACHE_CONTROL], HASHED_ASSET_CACHE_CONTROL);
        assert_eq!(body(response).await, WASM);
    }

    #[tokio::test]
    async fn an_unknown_path_is_served_the_index() {
        let response = serve_file(&DIR, "/substates/component_00");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "text/html");
        assert_eq!(response.headers()[CACHE_CONTROL], REVALIDATE_CACHE_CONTROL);
        assert_eq!(body(response).await, b"<html></html>");
    }

    #[tokio::test]
    async fn the_index_is_revalidated() {
        let response = serve_file(&DIR, "/index.html");

        assert_eq!(response.headers()[CACHE_CONTROL], REVALIDATE_CACHE_CONTROL);
    }
}
