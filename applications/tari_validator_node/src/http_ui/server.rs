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

use axum::{routing::get, Extension, Router};
use log::{error, info};

const LOG_TARGET: &str = "tari_validator_node::http_ui::server";

pub async fn run_http_ui_server(address: SocketAddr, json_rpc_address: Option<String>) -> Result<(), anyhow::Error> {
    let router = Router::new().route("/", get(index)).layer(Extension(Arc::new(
        json_rpc_address.unwrap_or_else(|| "127.0.0.1:18145".to_string()),
    )));

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

async fn index(Extension(json_rpc_address): Extension<Arc<String>>) -> axum::response::Html<String> {
    println!("address {:?}", json_rpc_address);
    include_str!("gui.html")
        .replace("{{address}}", &json_rpc_address)
        .into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_there_is_only_one_port_in_html() {
        assert_eq!(
            include_str!("gui.html").matches("{{address}}").count(),
            1,
            "There should be exactly one {{{{address}}}}"
        );
    }
}
