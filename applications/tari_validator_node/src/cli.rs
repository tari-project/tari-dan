//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::net::SocketAddr;

use clap::Parser;
use minotari_app_utilities::common_cli_args::CommonCliArgs;
use reqwest::Url;
use tari_common::configuration::{ConfigOverrideProvider, Network};
use tari_dan_app_utilities::p2p_config::ReachabilityMode;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Cli {
    #[clap(flatten)]
    pub common: CommonCliArgs,
    /// Enable tracing
    #[clap(long, aliases = &["tracing", "enable-tracing"])]
    pub tracing_enabled: bool,
    /// Bind address for JSON-rpc server
    #[clap(long, alias = "json-rpc-address")]
    pub json_rpc_listener_address: Option<SocketAddr>,
    #[clap(long, env = "TARI_VN_HTTP_UI_LISTENER_ADDRESS", alias = "http-ui-address")]
    pub http_ui_listener_address: Option<SocketAddr>,
    #[clap(long, env = "TARI_VN_JSON_RPC_PUBLIC_ADDRESS")]
    pub json_rpc_public_address: Option<String>,
    #[clap(long, alias = "node-grpc", short = 'g', env = "TARI_VN_MINOTARI_NODE_GRPC_URL")]
    pub minotari_node_grpc_url: Option<Url>,
    #[clap(long, short = 's')]
    pub peer_seeds: Vec<String>,
    #[clap(long)]
    pub listener_port: Option<u16>,
    /// Set the node to unreachable mode, the node will not act as a relay and will attempt relayed connections.
    #[clap(long)]
    pub reachability: Option<ReachabilityMode>,
    #[clap(long)]
    pub disable_mdns: bool,
    /// A replacement of a template address with a local WASM file, in the format <template_address>=<local file path>.
    /// FOR DEBUGGING PURPOSES ONLY
    #[clap(long, short = 'd')]
    pub debug_templates: Vec<String>,
}

impl ConfigOverrideProvider for Cli {
    fn get_config_property_overrides(&self, network: &mut Network) -> Vec<(String, String)> {
        let mut overrides = self.common.get_config_property_overrides(network);
        *network = self.common.network.unwrap_or(*network);
        overrides.push(("network".to_string(), network.to_string()));
        overrides.push(("validator_node.override_from".to_string(), network.to_string()));
        overrides.push(("p2p.seeds.override_from".to_string(), network.to_string()));

        if let Some(ref json_rpc_address) = self.json_rpc_listener_address {
            overrides.push((
                "validator_node.json_rpc_listener_address".to_string(),
                json_rpc_address.to_string(),
            ));
        }
        if let Some(ref ui_connect_address) = self.json_rpc_public_address {
            overrides.push((
                "validator_node.json_rpc_public_address".to_string(),
                ui_connect_address.to_string(),
            ));
        }
        if let Some(ref http_ui_address) = self.http_ui_listener_address {
            overrides.push((
                "validator_node.http_ui_listener_address".to_string(),
                http_ui_address.to_string(),
            ));
        }
        if !self.peer_seeds.is_empty() {
            overrides.push(("p2p.seeds.peer_seeds".to_string(), self.peer_seeds.join(",")));
        }
        if let Some(listener_port) = self.listener_port {
            overrides.push((
                "validator_node.p2p.listener_port".to_string(),
                listener_port.to_string(),
            ));
        }
        if let Some(reachability) = self.reachability {
            overrides.push((
                "validator_node.p2p.reachability_mode".to_string(),
                reachability.to_string(),
            ));
        }
        if self.disable_mdns {
            overrides.push(("validator_node.p2p.enable_mdns".to_string(), "false".to_string()));
        }
        if let Some(url) = self.minotari_node_grpc_url.as_ref() {
            overrides.push(("validator_node.base_node_grpc_url".to_string(), url.to_string()));
        }
        overrides
    }
}
