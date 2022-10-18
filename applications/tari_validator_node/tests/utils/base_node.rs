use std::{
    str::FromStr,
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};

use rand::rngs::OsRng;
use tari_base_node::{run_base_node, BaseNodeConfig, MetricsConfig};
use tari_common::configuration::CommonConfig;
use tari_comms::{multiaddr::Multiaddr, peer_manager::PeerFeatures, NodeIdentity};
use tari_comms_dht::DhtConfig;
use tari_p2p::{auto_update::AutoUpdateConfig, Network, PeerSeedsConfig};
use tempfile::tempdir;
use tokio::runtime;

use crate::TariWorld;

#[derive(Debug)]
pub struct BaseNodeProcess {
    pub name: String,
    pub port: u64,
    pub grpc_port: u64,
    pub identity: NodeIdentity,
    pub handle: JoinHandle<()>,
}

pub fn spawn_base_node(world: &mut TariWorld, bn_name: String) {
    // TODO: use different ports on each spawned base node
    let port = 9000;
    let grpc_port = 18152;
    let base_node_address = Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", port)).unwrap();
    let base_node_identity = NodeIdentity::random(&mut OsRng, base_node_address, PeerFeatures::COMMUNICATION_NODE);
    let identity = base_node_identity.clone();

    let handle = thread::spawn(move || {
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
        // FIXME: this option seems to be ignored by the base node, and only works with a real tor running
        base_node_config.base_node.use_libtor = true;
        base_node_config.base_node.grpc_enabled = true;
        base_node_config.base_node.grpc_address =
            Some(Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", grpc_port)).unwrap());
        base_node_config.base_node.data_dir = temp_dir.path().to_path_buf();
        base_node_config.base_node.identity_file = temp_dir.path().join("base_node_id.json");
        base_node_config.base_node.tor_identity_file = temp_dir.path().join("base_node_tor_id.json");

        base_node_config.base_node.lmdb_path = temp_dir.path().to_path_buf();
        base_node_config.base_node.p2p.datastore_path = temp_dir.path().to_path_buf();
        base_node_config.base_node.p2p.dht = DhtConfig::default_local_test();

        let mut builder = runtime::Builder::new_multi_thread();
        let rt = builder.enable_all().build().unwrap();

        let result = rt.block_on(run_base_node(Arc::new(base_node_identity), Arc::new(base_node_config)));
        if let Err(e) = result {
            println!("{:?}", e);
            panic!();
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
    thread::sleep(Duration::from_secs(5));
}
