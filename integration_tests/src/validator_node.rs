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
    fs,
    path::{Path, PathBuf},
};

use reqwest::Url;
use tari_common::configuration::{CommonConfig, StringList};
use tari_common_types::types::PublicKey;
use tari_dan_app_utilities::p2p_config::PeerSeedsConfig;
use tari_p2p::Network;
use tari_shutdown::Shutdown;
use tari_validator_node::{run_validator_node, ApplicationConfig, ValidatorNodeConfig, ValidatorRegistrationFile};
use tari_validator_node_client::ValidatorNodeClient;
use tari_wallet_daemon_client::types::KeyBranch;
use tokio::task;

use crate::{
    helpers::{check_join_handle, get_os_assigned_port, get_os_assigned_ports, wait_listener_on_local_port},
    indexer::spawn_indexer,
    logging::get_base_dir_for_scenario,
    wallet_daemon::spawn_wallet_daemon,
    TariWorld,
};

#[derive(Debug)]
pub struct ValidatorNodeProcess {
    pub name: String,
    pub public_key: PublicKey,
    pub port: u16,
    pub json_rpc_port: u16,
    pub http_ui_port: u16,
    pub base_node_grpc_port: u16,
    pub handle: task::JoinHandle<Result<(), anyhow::Error>>,
    pub temp_dir_path: PathBuf,
    pub shutdown: Shutdown,
}

impl ValidatorNodeProcess {
    pub fn create_client(&self) -> ValidatorNodeClient {
        get_vn_client(self.json_rpc_port)
    }

    pub async fn save_database(&self, database_name: String, to: &Path) {
        fs::create_dir_all(to).expect("Could not create directory");
        let from = &self.temp_dir_path.join(format!("{}.db", database_name));
        fs::copy(from, to.join(format!("{}.sqlite", database_name))).expect("Could not copy file");
    }

    pub fn get_registration_info(&self) -> ValidatorRegistrationFile {
        let registration_file_path = self.temp_dir_path.join("registration.json");
        let registration_file = fs::read_to_string(&registration_file_path).expect("Could not read file");
        serde_json::from_str(&registration_file).expect("Could not parse file")
    }
}

pub async fn spawn_validator_node(
    world: &mut TariWorld,
    validator_node_name: String,
    base_node_name: String,
    wallet_daemon_name: String,
    claim_fee_key_name: String,
) -> ValidatorNodeProcess {
    // each spawned VN will use different ports
    let (port, json_rpc_port) = get_os_assigned_ports();
    let http_ui_port = get_os_assigned_port();
    let base_node_grpc_port = world.base_nodes.get(&base_node_name).unwrap().grpc_port;
    let walletd = match world.wallet_daemons.get(&wallet_daemon_name) {
        Some(walletd) => walletd,
        None => {
            let indexer_name = format!("{}_indexer", wallet_daemon_name);
            if world.indexers.get(&indexer_name).is_none() {
                spawn_indexer(world, indexer_name.clone(), base_node_name).await;
            }
            spawn_wallet_daemon(world, wallet_daemon_name.clone(), indexer_name).await;
            world.wallet_daemons.get(&wallet_daemon_name).unwrap()
        },
    };
    let mut wallet_client = walletd.get_authed_client().await;

    // get the default wallet account public key
    let key = wallet_client.create_key(KeyBranch::Transaction).await.unwrap();
    world.wallet_keys.insert(claim_fee_key_name, key.id);

    // let wallet_account_pub = wallet_client.accounts_get_default().await.unwrap().public_key;
    let name = validator_node_name.clone();

    let peer_seeds: Vec<String> = world
        .vn_seeds
        .values()
        .map(|vn| format!("{}::/ip4/127.0.0.1/tcp/{}", vn.public_key, vn.port))
        .collect();

    let shutdown = Shutdown::new();
    let shutdown_signal = shutdown.to_signal();
    let temp_dir = get_base_dir_for_scenario(
        "validator_node",
        world.current_scenario_name.as_ref().unwrap(),
        &validator_node_name,
    );
    let temp_dir_path = temp_dir.clone();
    let enable_fees = world.fees_enabled;
    let handle = task::spawn(async move {
        let mut config = ApplicationConfig {
            common: CommonConfig::default(),
            validator_node: ValidatorNodeConfig::default(),
            peer_seeds: PeerSeedsConfig::default(),
            network: Network::LocalNet,
        };

        // temporal folder for the VN files (e.g. sqlite file, json files, etc.)
        println!("Using validator_node temp_dir: {}", temp_dir.display());
        config.common.base_path = temp_dir.clone();
        config.validator_node.data_dir = temp_dir.to_path_buf();
        config.validator_node.shard_key_file = temp_dir.join("shard_key.json");
        config.validator_node.identity_file = temp_dir.join("validator_node_id.json");
        config.validator_node.base_node_grpc_address = Some(format!("127.0.0.1:{}", base_node_grpc_port));

        // config.validator_node.public_address =
        // Some(config.validator_node.p2p.transport.tcp.listener_address.clone());
        config.validator_node.p2p.enable_mdns = false;
        config.validator_node.json_rpc_listener_address = Some(format!("127.0.0.1:{}", json_rpc_port).parse().unwrap());
        config.validator_node.http_ui_listener_address = Some(format!("127.0.0.1:{}", http_ui_port).parse().unwrap());
        config.validator_node.p2p.listener_port = port;

        config.validator_node.no_fees = !enable_fees;
        config.validator_node.fee_claim_public_key = key.public_key;

        // Add all other VNs as peer seeds
        config.peer_seeds.peer_seeds = StringList::from(peer_seeds);
        run_validator_node(&config, shutdown_signal).await
    });

    // Wait for node to start up
    let handle = wait_listener_on_local_port(handle, json_rpc_port).await;

    // Check if the inner thread panicked
    let handle = check_join_handle(&name, handle).await;

    // get the public key of the VN
    let public_key = get_vn_identity(json_rpc_port).await;

    // make the new vn able to be referenced by other processes
    ValidatorNodeProcess {
        name: name.clone(),
        public_key,
        port,
        base_node_grpc_port,
        http_ui_port,
        handle,
        json_rpc_port,
        temp_dir_path,
        shutdown,
    }
}

fn get_vn_client(port: u16) -> ValidatorNodeClient {
    let endpoint: Url = Url::parse(&format!("http://localhost:{}", port)).unwrap();
    ValidatorNodeClient::connect(endpoint).unwrap()
}

async fn get_vn_identity(jrpc_port: u16) -> PublicKey {
    // send the JSON RPC "get_identity" request to the VN
    let mut client = get_vn_client(jrpc_port);
    let resp = client.get_identity().await.unwrap();
    resp.public_key
}

impl ValidatorNodeProcess {
    pub fn stop(&mut self) {
        self.shutdown.trigger();
    }

    pub fn get_client(&self) -> ValidatorNodeClient {
        let endpoint: Url = Url::parse(&format!("http://localhost:{}", self.json_rpc_port)).unwrap();
        ValidatorNodeClient::connect(endpoint).unwrap()
    }
}
