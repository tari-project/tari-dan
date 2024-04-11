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
use tari_wallet_daemon_client::{ComponentAddressOrName, types::{AuthLoginAcceptRequest, AuthLoginRequest, AuthLoginResponse}, WalletDaemonClient};
use tokio::task;
use tari_wallet_daemon_client::types::{AccountsCreateFreeTestCoinsRequest, AccountsCreateRequest, KeyBranch};

use crate::{
    helpers::{check_join_handle, get_os_assigned_ports, wait_listener_on_local_port},
    logging::get_base_dir_for_scenario,
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

    let json_rpc_address = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), json_rpc_port);
    let signaling_server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), signaling_server_port);
    let indexer_url = format!("http://127.0.0.1:{}/json_rpc", indexer_jrpc_port);

    let mut config = ApplicationConfig {
        common: CommonConfig::default(),
        dan_wallet_daemon: WalletDaemonConfig::default(),
    };

    config.common.base_path = base_dir.clone();
    config.dan_wallet_daemon.json_rpc_address = Some(json_rpc_address);
    config.dan_wallet_daemon.signaling_server_address = Some(signaling_server_addr);
    config.dan_wallet_daemon.indexer_node_json_rpc_url = indexer_url;

    let handle = task::spawn(run_tari_dan_wallet_daemon(config, shutdown_signal));

    // Wait for node to start up
    let handle = wait_listener_on_local_port(handle, json_rpc_port).await;
    // Check if the task errored/panicked
    let _handle = check_join_handle(&wallet_daemon_name, handle).await;

    let wallet_daemon_process = DanWalletDaemonProcess {
        name: wallet_daemon_name.clone(),
        json_rpc_port,
        indexer_jrpc_port,
        temp_path_dir: base_dir,
        shutdown,
    };

    // if create_default_account {
    //     let mut client = wallet_daemon_process.get_authed_client().await;
    //     // Get the key that it will use for the first public key
    //    let key = client.create_specific_key(KeyBranch::Transaction, 0).await.unwrap();
    //    world.wallet_keys.insert(wallet_daemon_name.clone(), key.id);
    //     // let key_index = key_name.map(|k| {
    //     //     *world
    //     //         .wallet_keys
    //     //         .get(&k)
    //     //         .unwrap_or_else(|| panic!("Wallet {} not found", wallet_daemon_name))
    //     // });
    //     let request = AccountsCreateRequest {
    //         account_name: Some("default".to_string()),
    //         custom_access_rules: None,
    //         max_fee: None,
    //         is_default: true,
    //         key_id: Some(key.id),
    //     };
    //
    //     client.create_account(request).await.unwrap();
    // }
    world.wallet_daemons.insert(wallet_daemon_name, wallet_daemon_process);
}

impl DanWalletDaemonProcess {
    pub fn stop(&mut self) {
        self.shutdown.trigger();
    }

    pub fn get_client(&self) -> WalletDaemonClient {
        let endpoint = Url::parse(&format!("http://127.0.0.1:{}", self.json_rpc_port)).unwrap();
        WalletDaemonClient::connect(endpoint, None).unwrap()
    }

    pub async fn get_authed_client(&self) -> WalletDaemonClient {
        let mut client = self.get_client();
        // authentication
        let AuthLoginResponse { auth_token } = client
            .auth_request(AuthLoginRequest {
                permissions: vec!["Admin".to_string()],
                duration: None,
            })
            .await
            .unwrap();
        let auth_response = client
            .auth_accept(AuthLoginAcceptRequest {
                auth_token,
                name: "Testing Token".to_string(),
            })
            .await
            .unwrap();
        client.set_auth_token(auth_response.permissions_token);
        client
    }
}
