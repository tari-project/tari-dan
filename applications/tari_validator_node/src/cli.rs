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

use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;
use tari_common::{
    ConfigPath,
    configuration::{ConfigOverrideProvider, Network as L1Network},
};
use tari_ootle_app_utilities::{
    common_cli_args::CommonCliArgs,
    p2p_config::{PeerSeedsConfig, ReachabilityMode},
};
use tari_ootle_transaction::Network;
use url::Url;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
#[allow(clippy::struct_excessive_bools)]
pub struct Cli {
    #[clap(flatten)]
    pub common: CommonCliArgs,
    /// Enable tracing
    #[clap(long, aliases = &["tracing", "enable-tracing"])]
    pub tracing_enabled: bool,
    /// Bind address for JSON-rpc server
    #[clap(long, alias = "json-rpc-address")]
    pub json_rpc_listener_address: Option<SocketAddr>,
    /// Bind address for the Prometheus metrics server
    #[clap(long)]
    pub metrics_listener_address: Option<SocketAddr>,
    /// Allow cross-origin browser requests to the JSON-RPC
    #[clap(long)]
    pub enable_permissive_cors: bool,
    /// A host name, beyond localhost and IP addresses, that requests may address the JSON-RPC and web UI by (e.g. the
    /// name a reverse proxy forwards). Repeat it, or separate names with commas. `*` accepts every name.
    #[clap(long = "json-rpc-allowed-host", value_delimiter = ',')]
    pub json_rpc_allowed_hosts: Vec<String>,
    #[clap(long, alias = "node-grpc", short = 'g', env = "TARI_VN_MINOTARI_NODE_GRPC_URL")]
    pub epoch_oracle_minotari_node_grpc_url: Option<Url>,
    #[clap(long, alias = "oracle-config")]
    pub epoch_oracle_config: Option<PathBuf>,
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
    #[clap(long, env = "TARI_VN_TOKIO_CONSOLE_PORT")]
    pub tokio_console_port: Option<u16>,
    /// Skip the startup state sync check and go directly into consensus. The sync check will always report up-to-date.
    /// Intended for local development and recovery; do not use against a network where this node is actually behind.
    #[clap(long)]
    pub skip_sync: bool,
    /// Start even when this binary schedules a schema activation at an epoch the node has already
    /// passed. Diverges from the network; intended only for a node whose state is being discarded.
    #[clap(long)]
    pub allow_past_protocol_activation: bool,
    #[clap(subcommand)]
    pub command: Option<Subcommand>,
}

impl Cli {
    pub fn network(&self) -> Network {
        self.common.network
    }
}

impl ConfigOverrideProvider for Cli {
    fn get_config_property_overrides(&self, network: &L1Network) -> Vec<(String, String)> {
        let mut overrides = self.common.get_config_property_overrides(network);
        overrides.push(("network".to_string(), network.to_string()));
        overrides.push(("validator_node.override_from".to_string(), network.to_string()));
        overrides.push(("epoch_oracle.override_from".to_string(), network.to_string()));
        overrides.push(("p2p.override_from".to_string(), network.to_string()));
        overrides.push(("p2p.seeds.override_from".to_string(), network.to_string()));

        if let Some(ref json_rpc_address) = self.json_rpc_listener_address {
            overrides.push((
                "validator_node.json_rpc_listener_address".to_string(),
                json_rpc_address.to_string(),
            ));
        }

        if let Some(ref address) = self.metrics_listener_address {
            overrides.push((
                "validator_node.metrics_listener_address".to_string(),
                address.to_string(),
            ));
        }

        if self.enable_permissive_cors {
            overrides.push(("validator_node.enable_permissive_cors".to_string(), "true".to_string()));
        }
        if !self.json_rpc_allowed_hosts.is_empty() {
            overrides.push((
                "validator_node.json_rpc_allowed_hosts".to_string(),
                self.json_rpc_allowed_hosts.join(","),
            ));
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
        if self.allow_past_protocol_activation {
            overrides.push((
                "validator_node.allow_past_protocol_activation".to_string(),
                "true".to_string(),
            ));
        }
        if self.skip_sync {
            overrides.push(("validator_node.consensus.skip_sync".to_string(), "true".to_string()));
        }
        if let Some(url) = self.epoch_oracle_minotari_node_grpc_url.as_ref() {
            overrides.push((
                "epoch_oracle.base_layer.base_node_grpc_url".to_string(),
                url.to_string(),
            ));
            // Network presets set this key in `[<network>.epoch_oracle.base_layer]`, which the loader merges over the
            // root section after these overrides are applied.
            overrides.push((
                format!("{}.epoch_oracle.base_layer.base_node_grpc_url", network),
                url.to_string(),
            ));
        }
        // `epoch_oracle_config` is applied after the config is loaded (see main) so it can override an inline
        // `config_json` shipped in a network preset without tripping the "both set" error.
        overrides
    }
}

#[derive(clap::Subcommand, Debug)]
pub enum Subcommand {
    /// Start the validator node
    Start,
    /// Compact the database and exit
    CompactDb,
    /// Generate an identity keypair if one does not exist and exit
    GenerateIdentity,
}

#[cfg(test)]
mod tests {
    use tari_ootle_app_utilities::configuration::{
        get_default_config,
        load_configuration_with_overrides,
        write_config_to,
    };
    use tari_validator_node::ApplicationConfig;

    use super::*;

    #[test]
    fn grpc_url_flag_overrides_the_network_preset() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        write_config_to(&config_path, &get_default_config()).unwrap();
        let cli = Cli::parse_from([
            "tari_validator_node",
            "--network",
            "esmeralda",
            "-g",
            "http://my-node:18142",
        ]);

        let cfg = load_configuration_with_overrides(&config_path, &cli, Some(cli.network())).unwrap();
        let config = ApplicationConfig::load_from(&cfg).unwrap();

        assert_eq!(
            config.epoch_oracle.base_layer.base_node_grpc_url.unwrap().as_str(),
            "http://my-node:18142/"
        );
    }
}
