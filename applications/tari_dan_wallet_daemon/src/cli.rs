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

use std::{net::SocketAddr, path::PathBuf};

use anyhow::anyhow;
use clap::Parser;
use multiaddr::{Multiaddr, Protocol};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Cli {
    #[clap(long, alias = "endpoint", env = "JRPC_ENDPOINT")]
    pub listen_addr: Option<SocketAddr>,
    #[clap(long, alias = "signaling_server_address", env = "SIGNALING_SERVER_ADDRESS")]
    pub signaling_server_addr: Option<SocketAddr>,
    #[clap(long, short = 'b', alias = "basedir")]
    pub base_dir: Option<PathBuf>,
    #[clap(long, alias = "vn_url")]
    pub validator_node_endpoint: Option<Multiaddr>,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }

    pub fn listen_address(&self) -> SocketAddr {
        self.listen_addr
            .unwrap_or_else(|| SocketAddr::from(([127u8, 0, 0, 1], 9000)))
    }

    pub fn signaling_server_address(&self) -> SocketAddr {
        self.signaling_server_addr
            .unwrap_or_else(|| SocketAddr::from(([127u8, 0, 0, 1], 9100)))
    }

    pub fn base_dir(&self) -> PathBuf {
        self.base_dir
            .clone()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".tari/walletd"))
    }

    pub fn validator_node_endpoint(&self) -> String {
        self.validator_node_endpoint
            .as_ref()
            .map(multiaddr_to_http_url)
            .transpose()
            .unwrap()
            .unwrap_or_else(|| "http://127.0.0.1:18200/json_rpc".to_string())
    }
}

fn multiaddr_to_http_url(multiaddr: &Multiaddr) -> anyhow::Result<String> {
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

    Ok(format!("{}://{}:{}", scheme, ip, port))
}
