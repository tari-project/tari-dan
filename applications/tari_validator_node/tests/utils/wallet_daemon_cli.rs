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

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};

use tari_comms::multiaddr::Multiaddr;
use tari_dan_wallet_daemon::{cli::Cli, run_tari_dan_wallet_daemon};
use tari_dan_wallet_sdk::{DanWalletSdk, WalletSdkConfig};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;
use tari_shutdown::Shutdown;
use tari_wallet_daemon_client::WalletDaemonClient;
use tempfile::tempdir;

use crate::{
    utils::{helpers::get_os_assigned_ports, logging::get_base_dir},
    TariWorld,
};

#[derive(Debug)]
pub struct DanWalletDaemonProcess {
    pub name: String,
    pub port: u16,
    pub json_rpc_port: u16,
    pub validator_node_grpc_port: u16,
    pub temp_path_dir: String,
    pub shutdown: Shutdown,
}

pub fn spawn_wallet_daemon(world: &mut TariWorld, wallet_daemon_name: String, validator_node_name: String) {
    let (port, json_rpc_port) = get_os_assigned_ports();
    let base_dir = get_base_dir();

    let validator_node_grpc_port = world.validator_nodes.get(&validator_node_name).unwrap().port;
    let shutdown = Shutdown::new();
    let shutdown_signal = shutdown.to_signal();

    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), validator_node_grpc_port);
    let validator_node_endpoint = Multiaddr::new();

    let cli = Cli {
        listen_addr: Some(listen_addr),
        base_dir: Some(base_dir),
        validator_node_endpoint,
    };
}
