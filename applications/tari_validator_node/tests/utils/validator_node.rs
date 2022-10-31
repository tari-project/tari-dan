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

use std::{str::FromStr, time::Duration, path::PathBuf};

use reqwest::Url;
use tari_app_utilities::common_cli_args::CommonCliArgs;
use tari_common::configuration::CommonConfig;
use tari_comms::multiaddr::Multiaddr;
use tari_comms_dht::DhtConfig;
use tari_p2p::{Network, PeerSeedsConfig, TransportType};
use tari_validator_node::{cli::Cli, run_validator_node_with_cli, ApplicationConfig, ValidatorNodeConfig};
use tari_validator_node_client::ValidatorNodeClient;
use tempfile::tempdir;
use tokio::task;

use crate::TariWorld;

#[derive(Debug)]
pub struct ValidatorNodeProcess {
    pub name: String,
    pub port: u64,
    pub json_rpc_port: u64,
    pub http_ui_port: u64,
    pub base_node_grpc_port: u64,
    pub handle: task::JoinHandle<()>,
}

pub async fn spawn_validator_node(
    world: &mut TariWorld,
    validator_node_name: String,
    base_node_name: String,
    wallet_name: String,
) {
    // each spawned VN will use different ports
    let (port, json_rpc_port, http_ui_port) = match world.validator_nodes.values().last() {
        Some(v) => (v.port + 1, v.json_rpc_port + 1, v.http_ui_port + 1),
        None => (49000, 49500, 50000), // default ports if it's the first vn to be spawned
    };
    let base_node_grpc_port = world.base_nodes.get(&base_node_name).unwrap().grpc_port;
    let wallet_grpc_port = world.wallets.get(&wallet_name).unwrap().grpc_port;
    let name = validator_node_name.clone();

    let handle = task::spawn(async move {
        let mut config = ApplicationConfig {
            common: CommonConfig::default(),
            validator_node: ValidatorNodeConfig::default(),
            peer_seeds: PeerSeedsConfig::default(),
            network: Network::LocalNet,
        };

        // temporal folder for the VN files (e.g. sqlite file, json files, etc.)
        let temp_dir = tempdir().unwrap().path().join(validator_node_name.clone());
        println!("Using validator_node temp_dir: {}", temp_dir.display());
        config.validator_node.data_dir = temp_dir.to_path_buf();
        config.validator_node.shard_key_file = temp_dir.join("shard_key.json");
        config.validator_node.identity_file = temp_dir.join("validator_node_id.json");
        config.validator_node.tor_identity_file = temp_dir.join("validator_node_tor_id.json");
        config.validator_node.base_node_grpc_address =
            Some(format!("127.0.0.1:{}", base_node_grpc_port).parse().unwrap());
        config.validator_node.wallet_grpc_address = Some(format!("127.0.0.1:{}", wallet_grpc_port).parse().unwrap());

        config.validator_node.p2p.transport.transport_type = TransportType::Tcp;
        config.validator_node.p2p.transport.tcp.listener_address =
            Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", port)).unwrap();
        config.validator_node.p2p.public_address =
            Some(config.validator_node.p2p.transport.tcp.listener_address.clone());
        config.validator_node.p2p.datastore_path = temp_dir.to_path_buf().join("peer_db/wallet");
        config.validator_node.p2p.dht = DhtConfig::default_local_test();
        config.validator_node.json_rpc_address = Some(format!("127.0.0.1:{}", json_rpc_port).parse().unwrap());
        config.validator_node.http_ui_address = Some(format!("127.0.0.1:{}", http_ui_port).parse().unwrap());

        // The VNS will try to auto register upon startup
        config.validator_node.auto_register = true;

        let data_dir = config.validator_node.data_dir.clone();
        let data_dir_str = data_dir.clone().into_os_string().into_string().unwrap();
        let config_path = data_dir.join("config.toml");
        let cli = Cli {
            common: CommonCliArgs {
                base_path: data_dir_str,
                config: config_path.into_os_string().into_string().unwrap(),
                log_config: Some(get_config_file_path()),
                log_level: None,
                config_property_overrides: vec![],
            },
            tracing_enabled: true,
            network: Some(Network::LocalNet.to_string()),
            json_rpc_address: Some(format!("127.0.0.1:{}", json_rpc_port).parse().unwrap()),
        };

        let result = run_validator_node_with_cli(&config, &cli).await;
        if let Err(e) = result {
            panic!("{:?}", e);
        }
    });

    // make the new vn able to be referenced by other processes
    let validator_node_process = ValidatorNodeProcess {
        name: name.clone(),
        port,
        base_node_grpc_port,
        http_ui_port,
        handle,
        json_rpc_port,
    };
    world.validator_nodes.insert(name, validator_node_process);

    // We need to give it time for the VN to startup
    // TODO: it would be better to scan the VN to detect when it has started
    tokio::time::sleep(Duration::from_secs(5)).await;
    
    
}

fn get_config_file_path() -> PathBuf {
    let mut config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    config_path.push("tests/log4rs/validator_node.yml");
    config_path
}

pub async fn get_vn_client(port: u64) -> ValidatorNodeClient {
    let endpoint: Url = Url::parse(&format!("http://localhost:{}", port)).unwrap();
    ValidatorNodeClient::connect(endpoint).unwrap()
}
