// Copyright 2021. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{error::Error, path::PathBuf};

use anyhow::anyhow;
use multiaddr::{Multiaddr, Protocol};
use reqwest::Url;
use tari_validator_node_cli::{cli::Cli, command::Command, key_manager::KeyManager};
use tari_validator_node_client::ValidatorNodeClient;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::init();

    let endpoint = cli
        .vn_daemon_jrpc_endpoint
        .unwrap_or_else(|| "/ip4/127.0.0.1/tcp/18200".parse().unwrap());
    let endpoint = multiaddr_to_http_url(endpoint)?;
    let base_dir = cli
        .base_dir
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".tari/vncli"));

    // Create a key
    let key_manager = KeyManager::init(&base_dir)?;
    if key_manager.count() == 0 {
        key_manager.create()?;
    }

    log::info!("🌍️ Connecting to {}", endpoint);
    let client = ValidatorNodeClient::connect(endpoint)?;

    if let Err(err) = handle_command(cli.command, base_dir, client).await {
        eprintln!("👮 Command failed with error \"{}\"", err);
    }

    Ok(())
}

async fn handle_command(command: Command, base_dir: PathBuf, client: ValidatorNodeClient) -> anyhow::Result<()> {
    match command {
        Command::Vn(cmd) => cmd.handle(client).await?,
        Command::Templates(cmd) => cmd.handle(client).await?,
        Command::Keys(cmd) => cmd.handle(base_dir).await?,
        Command::Transactions(cmd) => cmd.handle(base_dir, client).await?,
        Command::Accounts(cmd) => cmd.handle(base_dir, client).await?,
        Command::Manifests(cmd) => cmd.handle()?,
        Command::Peers(cmd) => cmd.handle(client).await?,
        Command::Debug(cmd) => cmd.handle(client).await?,
    }

    Ok(())
}

pub fn multiaddr_to_http_url(multiaddr: Multiaddr) -> anyhow::Result<Url> {
    let mut iter = multiaddr.iter();
    let ip = iter.next().ok_or_else(|| anyhow!("Invalid multiaddr"))?;
    let port = iter.next().ok_or_else(|| anyhow!("Invalid multiaddr"))?;
    let scheme = iter.next();

    let ip = match ip {
        Protocol::Ip4(ip) => ip.to_string(),
        Protocol::Ip6(ip) => ip.to_string(),
        Protocol::Dns4(ip) | Protocol::Dns(ip) | Protocol::Dnsaddr(ip) | Protocol::Dns6(ip) => ip.to_string(),
        _ => return Err(anyhow!("Invalid multiaddr")),
    };

    let port = match port {
        Protocol::Tcp(port) => port,
        _ => return Err(anyhow!("Invalid multiaddr")),
    };

    let scheme = match scheme {
        Some(Protocol::Http) => "http",
        Some(Protocol::Https) => "https",
        None => "http",
        _ => return Err(anyhow!("Invalid multiaddr")),
    };

    let url = Url::parse(&format!("{}://{}:{}", scheme, ip, port))?;
    Ok(url)
}
