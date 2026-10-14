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

use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;
use tari_common::{
    SubConfigPath,
    configuration::{ConfigOverrideProvider, Network as L1Network},
};
use tari_engine_types::substate::SubstateId;
use tari_ootle_app_utilities::{
    common_cli_args::CommonCliArgs,
    p2p_config::{PeerSeedsConfig, ReachabilityMode},
};
use tari_ootle_transaction::Network;
use url::Url;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Cli {
    #[clap(flatten)]
    pub common: CommonCliArgs,

    /// Start even when this binary's schema activation schedule disagrees with the one the node has
    /// already run under. Diverges from the network; intended only for a node whose state is being
    /// discarded.
    #[clap(long)]
    pub allow_past_protocol_activation: bool,

    #[clap(long, short = 'a', multiple_values = true)]
    pub address: Vec<SubstateId>,

    #[clap(long, short = 'i')]
    pub scanning_interval: Option<u64>,

    /// Bind address for API server
    #[clap(long, short = 'r', alias = "api-address")]
    pub api_listen_address: Option<SocketAddr>,
    #[clap(long, alias = "node-grpc", short = 'g', env = "TARI_INDEXER_MINOTARI_NODE_GRPC_URL")]
    pub epoch_oracle_minotari_node_grpc_url: Option<Url>,
    #[clap(long, alias = "oracle-config")]
    pub epoch_oracle_config: Option<PathBuf>,
    // Networking
    #[clap(long, short = 's')]
    pub peer_seeds: Vec<String>,
    /// Port to listen on for p2p connections
    #[clap(long)]
    pub listener_port: Option<u16>,
    /// Set the node to unreachable mode, the node will not act as a relay and will attempt relayed connections.
    #[clap(long)]
    pub reachability: Option<ReachabilityMode>,
    #[clap(long)]
    pub disable_mdns: bool,
    /// Public address of the API
    #[clap(long, env = "TARI_INDEXER_WEB_UI_PUBLIC_API_URL")]
    pub web_ui_public_api_url: Option<String>,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }

    pub fn network(&self) -> Network {
        self.common.network
    }
}

impl ConfigOverrideProvider for Cli {
    fn get_config_property_overrides(&self, network: &L1Network) -> Vec<(String, String)> {
        let mut overrides = self.common.get_config_property_overrides(network);
        overrides.push(("network".to_string(), network.to_string()));
        overrides.push(("indexer.override_from".to_string(), network.to_string()));
        if self.allow_past_protocol_activation {
            overrides.push(("indexer.allow_past_protocol_activation".to_string(), "true".to_string()));
        }
        overrides.push(("epoch_oracle.override_from".to_string(), network.to_string()));
        overrides.push(("p2p.override_from".to_string(), network.to_string()));
        overrides.push(("p2p.seeds.override_from".to_string(), network.to_string()));

        if let Some(ref addr) = self.api_listen_address {
            overrides.push(("indexer.api_listen_address".to_string(), addr.to_string()));
        }

        if !self.peer_seeds.is_empty() {
            overrides.push((
                format!("{}.peer_seeds", PeerSeedsConfig::main_key_prefix()),
                self.peer_seeds.join(","),
            ));
            overrides.push((
                format!("{}.{}.peer_seeds", network, PeerSeedsConfig::main_key_prefix()),
                self.peer_seeds.join(","),
            ));
        }
        if let Some(listener_port) = self.listener_port {
            overrides.push(("indexer.p2p.listener_port".to_string(), listener_port.to_string()));
        }
        if let Some(seconds) = self.scanning_interval {
            overrides.push(("indexer.scanning_interval".to_string(), seconds.to_string()));
        }
        if let Some(ref api_url) = self.web_ui_public_api_url {
            overrides.push(("indexer.web_ui_public_api_url".to_string(), api_url.to_string()));
        }
        if let Some(reachability) = self.reachability {
            overrides.push(("indexer.p2p.reachability_mode".to_string(), reachability.to_string()));
        }
        if self.disable_mdns {
            overrides.push(("indexer.p2p.enable_mdns".to_string(), "false".to_string()));
        }
        if let Some(url) = self.epoch_oracle_minotari_node_grpc_url.as_ref() {
            overrides.push((
                "epoch_oracle.base_layer.base_node_grpc_url".to_string(),
                url.to_string(),
            ));
        }
        // `epoch_oracle_config` is applied after the config is loaded (see main) so it can override an inline
        // `config_json` shipped in a network preset without tripping the "both set" error.
        overrides
    }
}
