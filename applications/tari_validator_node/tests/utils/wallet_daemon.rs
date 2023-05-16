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
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use reqwest::Url;
use tari_common::configuration::CommonConfig;
use tari_dan_wallet_daemon::{
    config::{ApplicationConfig, WalletDaemonConfig},
    run_tari_dan_wallet_daemon,
};
use tari_shutdown::Shutdown;
use tari_wallet_daemon_client::WalletDaemonClient;
use tokio::task;

use crate::{
    utils::{
        helpers::{check_join_handle, get_os_assigned_ports, wait_listener_on_local_port},
        logging::get_base_dir_for_scenario,
    },
    TariWorld,
};

#[derive(Debug)]
pub struct DanWalletDaemonProcess {
    pub name: String,
    pub json_rpc_port: u16,
    pub indexer_jrpc_port: u16,
    pub temp_path_dir: PathBuf,
    pub shutdown: Shutdown,
}

pub async fn spawn_wallet_daemon(world: &mut TariWorld, wallet_daemon_name: String, indexer_name: String) {
    let (signaling_server_port, json_rpc_port) = get_os_assigned_ports();
    let base_dir = get_base_dir_for_scenario(
        "wallet_daemon",
        world.current_scenario_name.as_ref().unwrap(),
        &wallet_daemon_name,
    );

    let indexer_jrpc_port = world.indexers.get(&indexer_name).unwrap().json_rpc_port;
    let shutdown = Shutdown::new();
    let shutdown_signal = shutdown.to_signal();

    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), json_rpc_port);
    let signaling_server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), signaling_server_port);
    let indexer_url = format!("http://127.0.0.1:{}/json_rpc", indexer_jrpc_port);

    let mut config = ApplicationConfig {
        common: CommonConfig::default(),
        dan_wallet_daemon: WalletDaemonConfig::default(),
    };

    config.common.base_path = base_dir.clone();
    config.dan_wallet_daemon.listen_addr = Some(listen_addr);
    config.dan_wallet_daemon.signaling_server_addr = Some(signaling_server_addr);
    config.dan_wallet_daemon.indexer_node_json_rpc_url = indexer_url;

    let handle = task::spawn(run_tari_dan_wallet_daemon(config, shutdown_signal));

    // Wait for node to start up
    wait_listener_on_local_port(json_rpc_port).await;
    // Check if the task errored/panicked
    let _handle = check_join_handle(&wallet_daemon_name, handle).await;

    let wallet_daemon_process = DanWalletDaemonProcess {
        name: wallet_daemon_name.clone(),
        json_rpc_port,
        indexer_jrpc_port,
        temp_path_dir: base_dir,
        shutdown,
    };

    world.wallet_daemons.insert(wallet_daemon_name, wallet_daemon_process);
}

pub async fn get_walletd_client(port: u16) -> WalletDaemonClient {
    let endpoint: Url = Url::parse(&format!("http://127.0.0.1:{}", port)).unwrap();
    WalletDaemonClient::connect(endpoint, None).unwrap()
}

impl DanWalletDaemonProcess {
    pub fn stop(&mut self) {
        self.shutdown.trigger();
    }
}
