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
    env,
    path::{Path, PathBuf},
    str::FromStr,
    thread,
    thread::JoinHandle,
    time::Duration,
};

use log::Level;
use tari_app_grpc::{
    authentication::ClientAuthenticationInterceptor,
    tari_rpc::{wallet_client::WalletClient, ConnectivityStatus, Empty, GetIdentityRequest, SetBaseNodeRequest},
};
use tari_app_utilities::common_cli_args::CommonCliArgs;
use tari_common::configuration::CommonConfig;
use tari_common_types::grpc_authentication::GrpcAuthentication;
use tari_comms::multiaddr::Multiaddr;
use tari_comms_dht::{DbConnectionUrl, DhtConfig};
use tari_console_wallet::{run_wallet_with_cli, ApplicationConfig};
use tari_p2p::{auto_update::AutoUpdateConfig, Network, PeerSeedsConfig, TransportType};
use tari_shutdown::Shutdown;
use tari_wallet::WalletConfig;
use tokio::{runtime, runtime::Runtime};
use tonic::{
    codegen::InterceptedService,
    transport::{Channel, Endpoint},
};

type WalletGrpcClient = WalletClient<InterceptedService<Channel, ClientAuthenticationInterceptor>>;
use crate::{
    utils::{
        helpers::{get_os_assigned_ports, wait_listener_on_local_port},
        logging::get_base_dir_for_scenario,
    },
    TariWorld,
};

#[derive(Debug)]
pub struct WalletProcess {
    pub name: String,
    pub port: u16,
    pub grpc_port: u16,
    pub handle: JoinHandle<()>,
    pub temp_dir_path: PathBuf,
    pub shutdown: Shutdown,
}

impl WalletProcess {
    pub async fn create_client(&self) -> WalletGrpcClient {
        let wallet_addr = format!("http://127.0.0.1:{}", self.grpc_port);
        let endpoint = Endpoint::from_str(&wallet_addr).unwrap();
        let mut attempts = 0;
        let channel = loop {
            if self.handle.is_finished() {
                panic!("Wallet thread has ended");
            }
            match endpoint.connect().await {
                Ok(channel) => break channel,
                Err(e) => {
                    eprintln!(
                        "Attempt: {}/10 Could not connect to wallet GRPC address {}: {}",
                        attempts, wallet_addr, e
                    );
                    if attempts > 10 {
                        panic!("Failed to connect to wallet GRPC address {}", wallet_addr);
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    attempts += 1;
                },
            }
        };
        WalletClient::with_interceptor(
            channel,
            ClientAuthenticationInterceptor::create(&GrpcAuthentication::default()).unwrap(),
        )
    }
}

pub async fn spawn_wallet(world: &mut TariWorld, wallet_name: String, base_node_name: String) {
    // each spawned wallet will use different ports
    let (port, grpc_port) = get_os_assigned_ports();
    // let (port, grpc_port) = match world.base_nodes.values().last() {
    //     Some(v) => (v.port + 1, v.grpc_port + 1),
    //     None => (48000, 48500), // default ports if it's the first wallet to be spawned
    // };
    let base_node_public_key = world
        .base_nodes
        .get(&base_node_name)
        .unwrap()
        .identity
        .public_key()
        .clone();
    let base_node_port = world.base_nodes.get(&base_node_name).unwrap().port;
    let set_base_node_request = SetBaseNodeRequest {
        net_address: format! {"/ip4/127.0.0.1/tcp/{}", base_node_port},
        public_key_hex: base_node_public_key.to_string(),
    };
    let temp_dir = get_base_dir_for_scenario(
        "console_wallet",
        world.current_scenario_name.as_ref().unwrap(),
        &wallet_name,
    );
    let temp_dir_path = temp_dir.clone();
    let shutdown = Shutdown::new();

    let handle = thread::spawn({
        let mut shutdown = shutdown.clone();
        move || {
            let mut wallet_config = tari_console_wallet::ApplicationConfig {
                common: CommonConfig::default(),
                auto_update: AutoUpdateConfig::default(),
                wallet: WalletConfig::default(),
                peer_seeds: PeerSeedsConfig::default(),
            };

            eprintln!("Using wallet temp_dir: {}", temp_dir.display());

            wallet_config.wallet.network = Network::LocalNet;
            wallet_config.wallet.password = Some("test".into());
            wallet_config.wallet.grpc_enabled = true;
            wallet_config.wallet.grpc_address =
                Some(Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", grpc_port)).unwrap());
            wallet_config.wallet.data_dir = temp_dir.join("data/wallet");
            wallet_config.wallet.db_file = temp_dir.join("db/console_wallet.db");

            wallet_config.wallet.p2p.transport.transport_type = TransportType::Tcp;
            wallet_config.wallet.p2p.transport.tcp.listener_address =
                Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", port)).unwrap();
            wallet_config.wallet.p2p.public_addresses =
                vec![wallet_config.wallet.p2p.transport.tcp.listener_address.clone()].into();
            wallet_config.wallet.p2p.datastore_path = temp_dir.join("peer_db/wallet");
            wallet_config.wallet.p2p.dht = DhtConfig {
                // Not all platforms support sqlite memory connection urls
                database_url: DbConnectionUrl::File(temp_dir.join("dht.sqlite")),
                ..DhtConfig::default_local_test()
            };

            wallet_config.wallet.custom_base_node = Some(format!(
                "{}::/ip4/127.0.0.1/tcp/{}",
                base_node_public_key, base_node_port
            ));

            let mut builder = runtime::Builder::new_multi_thread();
            let rt = builder.enable_all().build().unwrap();

            run_wallet(rt, &mut wallet_config, &mut shutdown);
        }
    });

    // make the new wallet able to be referenced by other processes
    let wallet_process = WalletProcess {
        name: wallet_name.clone(),
        port,
        grpc_port,
        handle,
        temp_dir_path,
        shutdown,
    };

    eprintln!(
        "Wallet {} GRPC listening on port {}",
        wallet_name, wallet_process.grpc_port
    );
    // Wait for node to start up
    wait_listener_on_local_port(grpc_port).await;

    let mut wallet_client = wallet_process.create_client().await;

    let identity = wallet_client
        .identify(GetIdentityRequest {})
        .await
        .unwrap()
        .into_inner();

    eprintln!("Wallet {} comms address: {}", wallet_name, identity.public_address);

    // TODO: Clean up
    let mut status = wallet_client.get_network_status(Empty {}).await.unwrap().into_inner();
    let mut counter = 0;
    while status.status != ConnectivityStatus::Online as i32 {
        eprintln!(
            "Waiting for wallet to connect to base node {} {} {} (status: {:?})",
            base_node_name, set_base_node_request.public_key_hex, set_base_node_request.net_address, status
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
        counter += 1;
        if counter > 10 {
            panic!("Wallet failed to connect to base node");
        }
        status = wallet_client.get_network_status(Empty {}).await.unwrap().into_inner();
    }

    world.wallets.insert(wallet_name.clone(), wallet_process);
}

pub fn run_wallet(runtime: Runtime, config: &mut ApplicationConfig, shutdown: &mut Shutdown) {
    let data_dir = config.wallet.data_dir.clone();
    let data_dir_str = data_dir.clone().into_os_string().into_string().unwrap();

    let mut config_path = data_dir;
    config_path.push("config.toml");

    let log_config = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/log4rs/wallet.yml");

    let cli = tari_console_wallet::Cli {
        common: CommonCliArgs {
            base_path: data_dir_str,
            config: config_path.into_os_string().into_string().unwrap(),
            log_config: Some(log_config),
            log_level: Some(Level::Debug),
            config_property_overrides: vec![],
            network: None,
        },
        password: None,
        change_password: false,
        recovery: false,
        seed_words: None,
        seed_words_file_name: None,
        non_interactive_mode: true,
        input_file: None,
        command: None,
        wallet_notify: None,
        command_mode_auto_exit: false,
        grpc_enabled: true,
        grpc_address: None,
        command2: None,
    };

    if let Err(err) = run_wallet_with_cli(shutdown, runtime, config, cli) {
        eprintln!("Wallet error: {}", err);
    }
}
