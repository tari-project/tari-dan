use std::{str::FromStr, sync::Arc, thread, time::Duration};

use rand::rngs::OsRng;
use tari_base_node::{run_base_node, BaseNodeConfig, MetricsConfig};
use tari_common::configuration::CommonConfig;
use tari_comms::{multiaddr::Multiaddr, peer_manager::PeerFeatures, NodeIdentity};
use tari_comms_dht::DhtConfig;
use tari_p2p::{auto_update::AutoUpdateConfig, Network, PeerSeedsConfig};
use tempfile::tempdir;
use tokio::runtime;

pub fn spawn_base_node() {
    thread::spawn(move || {
        let base_node_address = Multiaddr::from_str("/ip4/127.0.0.1/tcp/9000").unwrap();
        let base_node_identity = NodeIdentity::random(&mut OsRng, base_node_address, PeerFeatures::COMMUNICATION_NODE);

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
        base_node_config.base_node.grpc_address = Some(Multiaddr::from_str("/ip4/127.0.0.1/tcp/18152").unwrap());
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

    // We need to give it time for the base node to startup
    // TODO: it would be better to scan the base node to detect when it has started
    thread::sleep(Duration::from_secs(2));
}
