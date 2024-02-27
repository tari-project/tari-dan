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

use std::{path::PathBuf, str::FromStr, sync::Arc};

use minotari_node::{run_base_node, BaseNodeConfig};
use rand::rngs::OsRng;
use tari_base_node_client::grpc::GrpcBaseNodeClient;
use tari_common::{configuration::CommonConfig, exit_codes::ExitError};
use tari_comms::{multiaddr::Multiaddr, peer_manager::PeerFeatures, NodeIdentity};
use tari_comms_dht::{DbConnectionUrl, DhtConfig};
use tari_p2p::{auto_update::AutoUpdateConfig, peer_seeds::SeedPeer, Network, PeerSeedsConfig, TransportType};
use tari_shutdown::Shutdown;
use tokio::task;

use crate::{
    helpers::{get_os_assigned_ports, wait_listener_on_local_port},
    logging::get_base_dir_for_scenario,
    TariWorld,
};

#[derive(Debug)]
pub struct BaseNodeProcess {
    pub name: String,
    pub port: u16,
    pub grpc_port: u16,
    pub identity: NodeIdentity,
    pub handle: task::JoinHandle<Result<(), ExitError>>,
    pub temp_dir_path: PathBuf,
    pub shutdown: Shutdown,
}

impl BaseNodeProcess {
    pub fn create_client(&self) -> GrpcBaseNodeClient {
        get_base_node_client(self.grpc_port)
    }
}

pub async fn spawn_base_node(world: &mut TariWorld, bn_name: String) {
    // each spawned base node will use different ports
    let (port, grpc_port) = get_os_assigned_ports();
    // let (port, grpc_port) = match world.base_nodes.values().last() {
    // Some(v) => (v.port + 1, v.grpc_port + 1),
    // None => (19000, 19500), // default ports if it's the first base node to be spawned
    // };
    let base_node_address = Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", port)).unwrap();
    let base_node_identity = NodeIdentity::random(&mut OsRng, base_node_address, PeerFeatures::COMMUNICATION_NODE);
    println!("Base node identity: {}", base_node_identity);
    let identity = base_node_identity.clone();
    let temp_dir = get_base_dir_for_scenario("base_node", world.current_scenario_name.as_ref().unwrap(), &bn_name);
    let temp_dir_path = temp_dir.clone();
    let base_node_name = bn_name.clone();

    let shutdown = Shutdown::new();
    let peer_seeds = world
        .base_nodes
        .values()
        .map(|bn| SeedPeer::new(bn.identity.public_key().clone(), bn.identity.public_addresses()).to_string())
        .collect::<Vec<_>>();

    let handle = task::spawn({
        let shutdown = shutdown.clone();
        async move {
            let mut base_node_config = minotari_node::ApplicationConfig {
                common: CommonConfig::default(),
                auto_update: AutoUpdateConfig::default(),
                base_node: BaseNodeConfig::default(),
                peer_seeds: PeerSeedsConfig {
                    peer_seeds: peer_seeds.into(),
                    ..Default::default()
                },
            };

            println!("Using base_node temp_dir: {}", temp_dir.display());
            base_node_config.common.base_path = temp_dir.clone();
            base_node_config.base_node.network = Network::LocalNet;
            base_node_config.base_node.grpc_enabled = true;
            base_node_config.base_node.grpc_address =
                Some(format!("/ip4/127.0.0.1/tcp/{}", grpc_port).parse().unwrap());
            base_node_config.base_node.report_grpc_error = true;

            base_node_config.base_node.data_dir = temp_dir.join("db");
            base_node_config.base_node.identity_file = temp_dir.join("base_node_id.json");
            base_node_config.base_node.tor_identity_file = temp_dir.join("base_node_tor_id.json");

            base_node_config.base_node.lmdb_path = temp_dir.to_path_buf();
            base_node_config.base_node.p2p.transport.transport_type = TransportType::Tcp;
            base_node_config.base_node.p2p.transport.tcp.listener_address =
                format!("/ip4/127.0.0.1/tcp/{}", port).parse().unwrap();
            base_node_config.base_node.p2p.public_addresses =
                vec![base_node_config.base_node.p2p.transport.tcp.listener_address.clone()].into();
            base_node_config.base_node.p2p.datastore_path = temp_dir.join("peer_db/base-node");
            base_node_config.base_node.p2p.dht = DhtConfig {
                // Not all platforms support sqlite memory connection urls
                database_url: DbConnectionUrl::File(temp_dir.join("dht.sqlite")),
                ..DhtConfig::default_local_test()
            };
            base_node_config.base_node.grpc_server_deny_methods = vec![];

            let result = run_base_node(shutdown, Arc::new(base_node_identity), Arc::new(base_node_config)).await;
            if let Err(e) = result {
                let dest = format!("./temp/base_node_{}", base_node_name);
                std::fs::create_dir_all(&dest).unwrap();
                std::fs::copy(temp_dir, dest).unwrap();
                return Err(e);
            }
            Ok(())
        }
    });

    // Wait for node to start up
    let handle = wait_listener_on_local_port(handle, grpc_port).await;
    // make the new base node able to be referenced by other processes
    let node_process = BaseNodeProcess {
        name: bn_name.clone(),
        port,
        grpc_port,
        identity,
        handle,
        temp_dir_path,
        shutdown,
    };

    world.base_nodes.insert(bn_name, node_process);
}

pub fn get_base_node_client(port: u16) -> GrpcBaseNodeClient {
    GrpcBaseNodeClient::new(format!("127.0.0.1:{}", port))
}
