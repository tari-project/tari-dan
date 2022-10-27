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

use std::{net::SocketAddr, str::FromStr, sync::Arc, time::Duration};

use rand::rngs::OsRng;
use tari_base_node::{run_base_node, BaseNodeConfig, MetricsConfig};
use tari_common::configuration::CommonConfig;
use tari_comms::{multiaddr::Multiaddr, peer_manager::PeerFeatures, NodeIdentity};
use tari_comms_dht::DhtConfig;
use tari_p2p::{auto_update::AutoUpdateConfig, Network, PeerSeedsConfig, TransportType};
use tari_validator_node::GrpcBaseNodeClient;
use tempfile::tempdir;
use tokio::task;

use crate::TariWorld;

#[derive(Debug)]
pub struct BaseNodeProcess {
    pub name: String,
    pub port: u64,
    pub grpc_port: u64,
    pub identity: NodeIdentity,
    pub handle: task::JoinHandle<()>,
}

pub async fn spawn_base_node(world: &mut TariWorld, bn_name: String) {
    // TODO: use different ports on each spawned base node
    let port = 48000;
    let grpc_port = 48152;
    let base_node_address = Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", port)).unwrap();
    let base_node_identity = NodeIdentity::random(&mut OsRng, base_node_address, PeerFeatures::COMMUNICATION_NODE);
    println!("Base node identity: {}", base_node_identity);
    let identity = base_node_identity.clone();

    let handle = task::spawn(async move {
        let mut base_node_config = tari_base_node::ApplicationConfig {
            common: CommonConfig::default(),
            auto_update: AutoUpdateConfig::default(),
            base_node: BaseNodeConfig::default(),
            peer_seeds: PeerSeedsConfig::default(),
            metrics: MetricsConfig::default(),
        };

        let temp_dir = tempdir().unwrap();
        println!("Using base_node temp_dir: {}", temp_dir.path().display());
        base_node_config.base_node.network = Network::LocalNet;
        base_node_config.base_node.grpc_enabled = true;
        base_node_config.base_node.grpc_address = Some(format!("/ip4/127.0.0.1/tcp/{}", grpc_port).parse().unwrap());
        base_node_config.base_node.report_grpc_error = true;

        base_node_config.base_node.data_dir = temp_dir.path().to_path_buf();
        base_node_config.base_node.identity_file = temp_dir.path().join("base_node_id.json");
        base_node_config.base_node.tor_identity_file = temp_dir.path().join("base_node_tor_id.json");

        base_node_config.base_node.lmdb_path = temp_dir.path().to_path_buf();
        base_node_config.base_node.p2p.transport.transport_type = TransportType::Tcp;
        base_node_config.base_node.p2p.transport.tcp.listener_address =
            format!("/ip4/127.0.0.1/tcp/{}", port).parse().unwrap();
        base_node_config.base_node.p2p.public_address =
            Some(base_node_config.base_node.p2p.transport.tcp.listener_address.clone());
        base_node_config.base_node.p2p.datastore_path = temp_dir.path().to_path_buf();
        base_node_config.base_node.p2p.dht = DhtConfig::default_local_test();

        let result = run_base_node(Arc::new(base_node_identity), Arc::new(base_node_config)).await;
        if let Err(e) = result {
            panic!("{:?}", e);
        }
    });

    // make the new base node able to be referenced by other processes
    let node_process = BaseNodeProcess {
        name: bn_name.clone(),
        port,
        grpc_port,
        identity,
        handle,
    };
    world.base_nodes.insert(bn_name, node_process);

    // We need to give it time for the base node to startup
    // TODO: it would be better to scan the base node to detect when it has started
    tokio::time::sleep(Duration::from_secs(5)).await;
}

pub async fn get_base_node_client(port: u64) -> GrpcBaseNodeClient {
    let endpoint: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
    GrpcBaseNodeClient::new(endpoint)
}
