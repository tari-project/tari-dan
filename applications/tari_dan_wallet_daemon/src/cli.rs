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
use minotari_app_utilities::common_cli_args::CommonCliArgs;
use tari_common::configuration::{ConfigOverrideProvider, Network};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Cli {
    #[clap(flatten)]
    pub common: CommonCliArgs,
    #[clap(long, alias = "endpoint", env = "JRPC_ENDPOINT")]
    pub json_rpc_address: Option<SocketAddr>,
    #[clap(long, env = "TARI_DAN_WALLET_UI_CONNECT_ADDRESS")]
    pub ui_connect_address: Option<String>,
    #[clap(long, env = "SIGNALING_SERVER_ADDRESS")]
    pub signaling_server_address: Option<SocketAddr>,
    #[clap(long, alias = "indexer-url")]
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
        if let Some(json_rpc_address) = self.json_rpc_address {
            overrides.push((
                "dan_wallet_daemon.json_rpc_address".to_string(),
                json_rpc_address.to_string(),
            ));
        }
        if let Some(ref ui_connect_address) = self.ui_connect_address {
            overrides.push((
                "dan_wallet_daemon.ui_connect_address".to_string(),
                ui_connect_address.to_string(),
            ));
        }
        if let Some(ref signaling_server_address) = self.signaling_server_address {
            overrides.push((
                "dan_wallet_daemon.signaling_server_address".to_string(),
                signaling_server_address.to_string(),
            ));
        }
        if let Some(ref indexer_node_json_rpc_url) = &self.indexer_node_json_rpc_url {
            overrides.push((
                "dan_wallet_daemon.indexer_node_json_rpc_url".to_string(),
                indexer_node_json_rpc_url.clone(),
            ));
        }
        overrides
    }
}
