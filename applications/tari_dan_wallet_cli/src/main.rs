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

use anyhow::anyhow;
use multiaddr::{Multiaddr, Protocol};
use reqwest::Url;
use tari_dan_wallet_cli::{cli::Cli, command::Command};
use tari_wallet_daemon_client::WalletDaemonClient;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::init();

    let endpoint = cli
        .daemon_jrpc_endpoint
        .unwrap_or_else(|| "/ip4/127.0.0.1/tcp/9000".parse().unwrap());
    let endpoint = multiaddr_to_http_url(endpoint)?;

    log::info!("🌍️ Connecting to {}", endpoint);
    let client = WalletDaemonClient::connect(endpoint, cli.token)?;

    if let Err(err) = handle_command(cli.command, client).await {
        eprintln!("👮 Command failed with error \"{}\"", err);
        return Err(err);
    }

    Ok(())
}

async fn handle_command(command: Command, client: WalletDaemonClient) -> anyhow::Result<()> {
    match command {
        // Command::Templates(cmd) => cmd.handle(client).await?,
        Command::Keys(cmd) => cmd.handle(client).await?,
        Command::Transactions(cmd) => cmd.handle(client).await?,
        Command::Accounts(cmd) => cmd.handle(client).await?,
        Command::Proofs(cmd) => cmd.handle(client).await?,
        Command::WebRtc(cmd) => cmd.handle(client).await?,
        Command::Auth(cmd) => cmd.handle(client).await?,
        Command::AccountNft(cmd) => cmd.handle(client).await?,
        Command::Validator(cmd) => cmd.handle(client).await?,
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
