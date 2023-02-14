//   Copyright 2023. The Tari Project
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

use std::{collections::HashMap, str::FromStr, time::Duration};

use reqwest::Url;
use tari_common::configuration::{CommonConfig, StringList};
use tari_comms::multiaddr::Multiaddr;
use tari_comms_dht::{DbConnectionUrl, DhtConfig};
use tari_engine_types::substate::{Substate, SubstateAddress};
use tari_indexer::{
    config::{ApplicationConfig, IndexerConfig},
    run_indexer,
    GetSubstateRequest,
};
use tari_indexer_client::IndexerClient;
use tari_p2p::{Network, PeerSeedsConfig, TransportType};
use tari_shutdown::Shutdown;
use tempfile::tempdir;
use tokio::task;

use crate::{utils::helpers::get_os_assigned_ports, TariWorld};

#[derive(Debug)]
pub struct IndexerProcess {
    pub name: String,
    pub port: u16,
    pub json_rpc_port: u16,
    pub base_node_grpc_port: u16,
    pub handle: task::JoinHandle<()>,
    pub temp_dir_path: String,
    pub shutdown: Shutdown,
}

impl IndexerProcess {
    pub async fn get_substate(&self, world: &TariWorld, output_ref: String, version: u32) -> Substate {
        let substate_address_map: HashMap<String, SubstateAddress> = world
            .outputs
            .iter()
            .flat_map(|(name, outputs)| {
                outputs
                    .iter()
                    .map(move |(child_name, addr)| (format!("{}/{}", name, child_name), addr.address.clone()))
            })
            .collect();
        let address = substate_address_map.get(&output_ref).unwrap().to_string();

        let params = GetSubstateRequest {
            address,
            version: Some(version),
        };

        let mut client = self.get_indexer_client().await;
        let resp: Substate = client.send_request("get_substate", params).await.unwrap();
        resp

        // let substate: Substate = serde_json::from_str(resp).unwrap();
        // substate
    }

    pub async fn get_indexer_client(&self) -> IndexerClient {
        let endpoint: Url = Url::parse(&format!("http://localhost:{}", self.json_rpc_port)).unwrap();
        IndexerClient::connect(endpoint).unwrap()
    }
}

pub async fn spawn_indexer(world: &mut TariWorld, indexer_name: String, base_node_name: String) {
    // each spawned indexer will use different ports
    let (port, json_rpc_port) = get_os_assigned_ports();

    let base_node_grpc_port = world.base_nodes.get(&base_node_name).unwrap().grpc_port;
    let name = indexer_name.clone();

    let temp_dir = tempdir().unwrap().path().join(indexer_name.clone());
    let temp_dir_path = temp_dir.display().to_string();

    // we need to add all the validator nodes as seed peers
    let peer_seeds: Vec<String> = world
        .validator_nodes
        .values()
        .map(|vn| format!("{}::/ip4/127.0.0.1/tcp/{}", vn.public_key, vn.port))
        .collect();

    let shutdown = Shutdown::new();
    let shutdown_signal = shutdown.to_signal();

    let handle = task::spawn(async move {
        let mut config = ApplicationConfig {
            common: CommonConfig::default(),
            peer_seeds: PeerSeedsConfig::default(),
            network: Network::LocalNet,
            indexer: IndexerConfig::default(),
        };

        // temporal folder for the VN files (e.g. sqlite file, json files, etc.)
        let temp_dir = tempdir().unwrap().path().join(indexer_name.clone());
        println!("Using indexer temp_dir: {}", temp_dir.display());
        config.indexer.data_dir = temp_dir.to_path_buf();
        config.indexer.identity_file = temp_dir.join("indexer_id.json");
        config.indexer.tor_identity_file = temp_dir.join("indexer_tor_id.json");
        config.indexer.base_node_grpc_address = Some(format!("127.0.0.1:{}", base_node_grpc_port).parse().unwrap());

        config.indexer.p2p.transport.transport_type = TransportType::Tcp;
        config.indexer.p2p.transport.tcp.listener_address =
            Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", port)).unwrap();
        config.indexer.p2p.public_address = Some(config.indexer.p2p.transport.tcp.listener_address.clone());
        config.indexer.public_address = Some(config.indexer.p2p.transport.tcp.listener_address.clone());
        config.indexer.p2p.datastore_path = temp_dir.to_path_buf().join("peer_db/vn");
        config.indexer.p2p.dht = DhtConfig {
            // Not all platforms support sqlite memory connection urls
            database_url: DbConnectionUrl::File(temp_dir.join("dht.sqlite")),
            ..DhtConfig::default_local_test()
        };
        config.indexer.json_rpc_address = Some(format!("127.0.0.1:{}", json_rpc_port).parse().unwrap());

        // Add all other VNs as peer seeds
        config.peer_seeds.peer_seeds = StringList::from(peer_seeds);

        let result = run_indexer(config, shutdown_signal).await;
        if let Err(e) = result {
            panic!("{:?}", e);
        }
    });

    // We need to give it time for the indexer to startup
    // TODO: it would be better to scan the VN to detect when it has started
    tokio::time::sleep(Duration::from_secs(5)).await;

    // make the new vn able to be referenced by other processes
    let indexer_process = IndexerProcess {
        name: name.clone(),
        port,
        base_node_grpc_port,
        handle,
        json_rpc_port,
        temp_dir_path,
        shutdown,
    };
    world.indexers.insert(name, indexer_process);
}
