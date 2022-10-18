use std::{
    thread::{self, JoinHandle},
    time::Duration,
};

use axum::http::HeaderMap;
use reqwest::{header, Response, Url};
use serde_json::json;
use tari_common::configuration::CommonConfig;
use tari_p2p::{Network, PeerSeedsConfig, TransportType};
use tari_validator_node::{run_node, ApplicationConfig, ValidatorNodeConfig};
use tempfile::tempdir;
use tokio::runtime;

use crate::TariWorld;

#[derive(Debug)]
pub struct ValidatorNodeProcess {
    pub name: String,
    pub handle: JoinHandle<()>,
}

pub fn spawn_validator_node(
    world: &mut TariWorld,
    validator_node_name: String,
    base_node_name: String,
    wallet_name: String,
) {
    let base_node_grpc_port = world.base_nodes.get(&base_node_name).unwrap().grpc_port;
    let wallet_grpc_port = world.wallets.get(&wallet_name).unwrap().grpc_port;

    let handle = thread::spawn(move || {
        // TODO: store the VN in the world by the name
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
        config.validator_node.base_node_grpc_address = format!("127.0.0.1:{}", base_node_grpc_port).parse().unwrap();
        config.validator_node.wallet_grpc_address = format!("127.0.0.1:{}", wallet_grpc_port).parse().unwrap();
        config.validator_node.p2p.transport.transport_type = TransportType::Tcp;

        let mut builder = runtime::Builder::new_multi_thread();
        let rt = builder.enable_all().build().unwrap();
        let result = rt.block_on(run_node(&config));
        if let Err(e) = result {
            println!("{:?}", e);
            panic!();
        }
    });

    // make the new vn able to be referenced by other processes
    let validator_node_process = ValidatorNodeProcess {
        name: validator_node_name.clone(),
        handle,
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
