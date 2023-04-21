// Copyright 2023. The Tari Project
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

use std::net::SocketAddr;

use clap::Parser;
use tari_app_utilities::common_cli_args::CommonCliArgs;
use tari_common::configuration::{ConfigOverrideProvider, Network};
use tari_engine_types::substate::SubstateAddress;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Cli {
    #[clap(flatten)]
    pub common: CommonCliArgs,

    /// Supply a network (overrides existing configuration)
    #[clap(long, env = "TARI_NETWORK")]
    pub cli_network: Option<String>,

    #[clap(long, short = 'a', multiple_values = true)]
    pub address: Vec<SubstateAddress>,

    #[clap(long, short = 'i')]
    pub dan_layer_scanning_internal: Option<u64>,

    /// Bind address for JSON-rpc server
    #[clap(long, short = 'r', alias = "rpc-address")]
    pub json_rpc_address: Option<SocketAddr>,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }
}

impl ConfigOverrideProvider for Cli {
    fn get_config_property_overrides(&self, default_network: Network) -> Vec<(String, String)> {
        let mut overrides = self.common.get_config_property_overrides(default_network);
        let network = self.common.network.clone().unwrap_or(default_network).to_string();
        overrides.push(("network".to_string(), network.clone()));
        overrides.push(("indexer.override_from".to_string(), network.clone()));
        overrides.push(("p2p.seeds.override_from".to_string(), network));

        if let Some(ref addr) = self.json_rpc_address {
            overrides.push(("indexer.json_rpc_address".to_string(), addr.to_string()));
        }
        if let Some(seconds) = self.dan_layer_scanning_internal {
            overrides.push(("indexer.dan_layer_scanning_interval".to_string(), seconds.to_string()));
        }
        overrides
    }
}
