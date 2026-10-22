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
    ops::ControlFlow,
    path::{Path, PathBuf},
    str::FromStr,
    thread,
    thread::JoinHandle,
    time::Duration,
};

use minotari_app_grpc::{
    authentication::ClientAuthenticationInterceptor,
    tari_rpc::{GetIdentityRequest, wallet_client::WalletClient},
};
use minotari_app_utilities::common_cli_args::CommonCliArgs;
use minotari_console_wallet::{ApplicationConfig, run_wallet_with_cli};
use minotari_wallet::WalletConfig;
use tari_common::{configuration::CommonConfig, exit_codes::ExitError};
use tari_common_sqlite::connection::DbConnectionUrl;
use tari_common_types::grpc_authentication::GrpcAuthentication;
use tari_comms::multiaddr::Multiaddr;
use tari_comms_dht::DhtConfig;
use tari_p2p::{Network, PeerSeedsConfig, TransportType, auto_update::AutoUpdateConfig};
use tari_shutdown::Shutdown;
use tokio::{runtime, runtime::Runtime};
use tonic::{
    codegen::InterceptedService,
    transport::{Channel, Endpoint},
};

type WalletGrpcClient = WalletClient<InterceptedService<Channel, ClientAuthenticationInterceptor>>;
use crate::{
    TariWorld,
    cucumber_log,
    helpers::{get_os_assigned_ports, wait_listener_on_local_port_os_thread},
    logging::get_base_dir_for_scenario,
    wait::wait_until,
};

#[derive(Debug)]
pub struct WalletProcess {
    pub name: String,
    pub port: u16,
    pub grpc_port: u16,
    pub handle: JoinHandle<Result<(), ExitError>>,
    pub temp_dir_path: PathBuf,
    pub shutdown: Shutdown,
}

impl WalletProcess {
    pub async fn create_client(&self) -> WalletGrpcClient {
        let wallet_addr = format!("http://127.0.0.1:{}", self.grpc_port);
        let endpoint = Endpoint::from_str(&wallet_addr).unwrap();
        let channel = wait_until(Duration::from_secs(12), async || {
            if self.handle.is_finished() {
                panic!("Wallet thread has ended");
            }
            match endpoint.connect().await {
                Ok(channel) => ControlFlow::Break(channel),
                Err(e) => {
                    cucumber_log!("Could not connect to wallet GRPC address {}: {}", wallet_addr, e);
                    ControlFlow::Continue(e.to_string())
                },
            }
        })
        .await
        .unwrap_or_else(|err| panic!("Failed to connect to wallet GRPC address {wallet_addr}: {err}"));
        WalletClient::with_interceptor(
            channel,
            ClientAuthenticationInterceptor::create(&GrpcAuthentication::default()).unwrap(),
        )
    }

    pub fn mark_point_in_logs(&self, world: &TariWorld, point: &str) {
        let temp_dir = get_base_dir_for_scenario(
            "console_wallet",
            world.current_scenario_name.as_ref().unwrap(),
            &self.name,
        );
        let log_file = temp_dir.join("console_wallet_point.log");
        if !log_file.exists() {
            std::fs::write(&log_file, point).unwrap();
        }
        let log_content = std::fs::read_to_string(&log_file).unwrap();
        let log_content = format!("{}\n{}", point, log_content);
        std::fs::write(&log_file, log_content).unwrap();
    }
}

pub async fn spawn_minotari_wallet(world: &mut TariWorld, wallet_name: String, base_node_name: String) {
    // each spawned wallet will use different ports
    let (port, grpc_port) = get_os_assigned_ports();

    let base_node_http_port = world.get_base_node(&base_node_name).http_port;
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
            let mut wallet_config = ApplicationConfig {
                common: CommonConfig::default(),
                auto_update: AutoUpdateConfig::default(),
                wallet: WalletConfig::default(),
                peer_seeds: PeerSeedsConfig::default(),
            };

            cucumber_log!("Using wallet temp_dir: {}", temp_dir.display());

            wallet_config.wallet.set_base_path(temp_dir.clone());
            wallet_config.wallet.network = Network::LocalNet;
            wallet_config.wallet.password = Some("test".into());
            wallet_config.wallet.grpc_enabled = true;
            wallet_config.wallet.grpc_address =
                Some(Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", grpc_port)).unwrap());
            wallet_config.wallet.data_dir = temp_dir.join("data/wallet");
            wallet_config.wallet.db_file = temp_dir.join("db/console_wallet.db");
            wallet_config.wallet.contacts_auto_ping_interval = Duration::from_secs(2);
            wallet_config
                .wallet
                .output_manager_service_config
                .num_confirmations_required = 1;
            wallet_config.wallet.output_manager_service_config.prevent_fee_gt_amount = false;
            wallet_config
                .wallet
                .base_node_service_config
                .base_node_monitor_max_refresh_interval = Duration::from_secs(15);
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
            wallet_config.wallet.http_server_url = format!("http://127.0.0.1:{}", base_node_http_port);

            let mut builder = runtime::Builder::new_multi_thread();
            let rt = builder.enable_all().build().unwrap();

            run_wallet(rt, &mut wallet_config, &mut shutdown)
        }
    });

    // Wait for node to start up
    let handle = wait_listener_on_local_port_os_thread("MinotariWallet", handle, grpc_port).await;

    // make the new wallet able to be referenced by other processes
    let wallet_process = WalletProcess {
        name: wallet_name.clone(),
        port,
        grpc_port,
        handle,
        temp_dir_path,
        shutdown,
    };

    // cucumber_log!(
    //     "Wallet {} GRPC listening on port {}",
    //     wallet_name, wallet_process.grpc_port
    // );

    let mut wallet_client = wallet_process.create_client().await;

    let _identity = wallet_client
        .identify(GetIdentityRequest {})
        .await
        .unwrap()
        .into_inner();

    crate::cucumber_log!("Minotari wallet {} started", wallet_name);
    world.wallets.insert(wallet_name, wallet_process);
}

pub fn run_wallet(runtime: Runtime, config: &mut ApplicationConfig, shutdown: &mut Shutdown) -> Result<(), ExitError> {
    let data_dir = config.wallet.data_dir.clone();
    let data_dir_str = data_dir.clone().into_os_string().into_string().unwrap();

    let mut config_path = data_dir;
    config_path.push("config.toml");

    let log_config = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/log4rs/wallet.yml");

    let cli = minotari_console_wallet::Cli {
        common: CommonCliArgs {
            base_path: data_dir_str,
            config: config_path.into_os_string().into_string().unwrap(),
            log_config: Some(log_config),
            log_path: None,
            config_property_overrides: vec![],
            network: None,
        },
        spend_key: None,
        view_private_key: None,
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
        profile_with_tokio_console: false,
        libtor_data_dir: None,
        birthday: None,
        skip_recovery: false,
        print_env: false,
        burn_proof_out: None,
        // In-process: the test harness owns shutdown, so the wallet must not install its own signal handlers.
        install_grpc_signal_handler: false,
    };

    run_wallet_with_cli(shutdown, runtime, config, cli)?;

    Ok(())
}
