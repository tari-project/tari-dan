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

use std::net::SocketAddr;

use clap::Parser;
use tari_app_utilities::common_cli_args::CommonCliArgs;
use tari_common::configuration::{ConfigOverrideProvider, Network};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Cli {
    #[clap(flatten)]
    pub common: CommonCliArgs,
    #[clap(long, alias = "endpoint", env = "JRPC_ENDPOINT")]
    pub listen_addr: Option<SocketAddr>,
    #[clap(long, alias = "signaling_server_address", env = "SIGNALING_SERVER_ADDRESS")]
    pub signaling_server_addr: Option<SocketAddr>,
    #[clap(long, alias = "indexer_url")]
    pub indexer_node_json_rpc_url: Option<String>,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }
}

impl ConfigOverrideProvider for Cli {
    fn get_config_property_overrides(&self, default_network: Network) -> Vec<(String, String)> {
        let mut overrides = self.common.get_config_property_overrides(default_network);
        if let Some(listen_addr) = self.listen_addr {
            overrides.push(("dan_wallet_daemon.listen_addr".to_string(), listen_addr.to_string()));
        }
        if let Some(signaling_server_addr) = self.signaling_server_addr {
            overrides.push((
                "dan_wallet_daemon.signaling_server_addr".to_string(),
                signaling_server_addr.to_string(),
            ));
        }
        if let Some(indexer_node_json_rpc_url) = &self.indexer_node_json_rpc_url {
            overrides.push((
                "dan_wallet_daemon.indexer_node_json_rpc_url".to_string(),
                indexer_node_json_rpc_url.clone(),
            ));
        }
        overrides
    }
}
