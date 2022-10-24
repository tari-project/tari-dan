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

use std::{
    str::FromStr,
    thread::{self, JoinHandle},
    time::Duration,
};

use axum::http::HeaderMap;
use reqwest::{header, Response, Url};
use serde_json::json;
use tari_app_utilities::common_cli_args::CommonCliArgs;
use tari_common::configuration::CommonConfig;
use tari_comms::multiaddr::Multiaddr;
use tari_comms_dht::DhtConfig;
use tari_p2p::{Network, PeerSeedsConfig, TransportType};
use tari_validator_node::{cli::Cli, run_validator_node_with_cli, ApplicationConfig, ValidatorNodeConfig};
use tempfile::tempdir;
use tokio::runtime;

use crate::TariWorld;

#[derive(Debug)]
pub struct ValidatorNodeProcess {
    pub name: String,
    pub port: u64,
    pub json_rpc_port: u64,
    pub handle: JoinHandle<()>,
}

pub fn spawn_validator_node(
    world: &mut TariWorld,
    validator_node_name: String,
    base_node_name: String,
    wallet_name: String,
) {
    // TODO: use different ports on each spawned vn
    let port = 8003;
    let json_rpc_port = 18145;
    let base_node_grpc_port = world.base_nodes.get(&base_node_name).unwrap().grpc_port;
    let wallet_grpc_port = world.wallets.get(&wallet_name).unwrap().grpc_port;

    let handle = thread::spawn(move || {
        let mut config = ApplicationConfig {
            common: CommonConfig::default(),
            validator_node: ValidatorNodeConfig::default(),
            peer_seeds: PeerSeedsConfig::default(),
            network: Network::LocalNet,
        };

        // temporal folder for the VN files (e.g. sqlite file, json files, etc.)
        let temp_dir = tempdir().unwrap();
        println!("Using validator_node temp_dir: {}", temp_dir.path().display());
        config.validator_node.data_dir = temp_dir.path().to_path_buf();
        config.validator_node.shard_key_file = temp_dir.path().join("shard_key.json");
        config.validator_node.identity_file = temp_dir.path().join("validator_node_id.json");
        config.validator_node.tor_identity_file = temp_dir.path().join("validator_node_tor_id.json");
        config.validator_node.base_node_grpc_address =
            Some(format!("127.0.0.1:{}", base_node_grpc_port).parse().unwrap());
        config.validator_node.wallet_grpc_address = Some(format!("127.0.0.1:{}", wallet_grpc_port).parse().unwrap());

        config.validator_node.p2p.transport.transport_type = TransportType::Tcp;
        config.validator_node.p2p.transport.tcp.listener_address =
            Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", port)).unwrap();
        config.validator_node.p2p.public_address =
            Some(config.validator_node.p2p.transport.tcp.listener_address.clone());
        config.validator_node.p2p.datastore_path = temp_dir.path().to_path_buf().join("peer_db/wallet");
        config.validator_node.p2p.dht = DhtConfig::default_local_test();
        config.validator_node.json_rpc_address = Some(format!("127.0.0.1:{}", json_rpc_port).parse().unwrap());

        let data_dir = config.validator_node.data_dir.clone();
        let data_dir_str = data_dir.clone().into_os_string().into_string().unwrap();
        let mut config_path = data_dir;
        config_path.push("config.toml");
        let cli = Cli {
            common: CommonCliArgs {
                base_path: data_dir_str,
                config: config_path.into_os_string().into_string().unwrap(),
                log_config: None,
                log_level: None,
                config_property_overrides: vec![],
            },
            tracing_enabled: true,
            network: Some(Network::LocalNet.to_string()),
            json_rpc_address: Some(format!("127.0.0.1:{}", json_rpc_port).parse().unwrap()),
        };

        let mut builder = runtime::Builder::new_multi_thread();
        let rt = builder.enable_all().build().unwrap();
        let result = rt.block_on(run_validator_node_with_cli(&config, &cli));
        if let Err(e) = result {
            println!("{:?}", e);
            panic!();
        }
    });

    // make the new vn able to be referenced by other processes
    let validator_node_process = ValidatorNodeProcess {
        name: validator_node_name.clone(),
        port,
        handle,
        json_rpc_port,
    };
    world
        .validator_nodes
        .insert(validator_node_name, validator_node_process);

    // We need to give it time for the VN to startup
    // TODO: it would be better to scan the VN to detect when it has started
    thread::sleep(Duration::from_secs(5));
}

pub async fn send_vn_json_rpc_request<T: Into<serde_json::Value>>(port: u64, method: String, body: T) -> Response {
    let endpoint: Url = Url::parse(&format!("http://localhost:{}", port)).unwrap();
    let client = reqwest::Client::builder()
        .default_headers({
            let mut headers = HeaderMap::with_capacity(1);
            headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
            headers
        })
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let request_json = json!(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": body.into(),
        }
    );
    client
        .post(endpoint.clone())
        .body(request_json.to_string())
        .send()
        .await
        .unwrap()
}
