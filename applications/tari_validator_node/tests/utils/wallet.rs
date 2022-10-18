use std::{
    str::FromStr,
    thread::{self, JoinHandle},
    time::Duration,
};

use tari_common::configuration::CommonConfig;
use tari_comms::multiaddr::Multiaddr;
use tari_comms_dht::DhtConfig;
use tari_console_wallet::run_wallet;
use tari_p2p::{auto_update::AutoUpdateConfig, Network, PeerSeedsConfig, TransportType};
use tari_wallet::WalletConfig;
use tempfile::tempdir;
use tokio::runtime;

use crate::TariWorld;

#[derive(Debug)]
pub struct WalletProcess {
    pub name: String,
    pub port: u64,
    pub grpc_port: u64,
    pub handle: JoinHandle<()>,
}

pub fn spawn_wallet(world: &mut TariWorld, wallet_name: String, base_node_name: String) {
    // TODO: use different ports on each spawned wallet
    let port = 9001;
    let grpc_port = 18153;
    let base_node_public_key = world
        .base_nodes
        .get(&base_node_name)
        .unwrap()
        .identity
        .public_key()
        .clone();
    let base_node_grpc_port = world.base_nodes.get(&base_node_name).unwrap().grpc_port;

    let handle = thread::spawn(move || {
        let mut wallet_config = tari_console_wallet::ApplicationConfig {
            common: CommonConfig::default(),
            auto_update: AutoUpdateConfig::default(),
            wallet: WalletConfig::default(),
            peer_seeds: PeerSeedsConfig::default(),
        };

        let temp_dir = tempdir().unwrap();
        println!("Using wallet temp_dir: {}", temp_dir.path().display());

        wallet_config.wallet.network = Network::LocalNet;
        wallet_config.wallet.password = Some("test".into());
        wallet_config.wallet.grpc_enabled = true;
        wallet_config.wallet.grpc_address =
            Some(Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", grpc_port)).unwrap());
        wallet_config.wallet.data_dir = temp_dir.path().to_path_buf().join("data/wallet");
        wallet_config.wallet.db_file = temp_dir.path().to_path_buf().join("db/console_wallet.db");

        wallet_config.wallet.p2p.transport.transport_type = TransportType::Tcp;
        wallet_config.wallet.p2p.transport.tcp.listener_address =
            Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", port)).unwrap();
        wallet_config.wallet.p2p.public_address = Some(wallet_config.wallet.p2p.transport.tcp.listener_address.clone());
        wallet_config.wallet.p2p.datastore_path = temp_dir.path().to_path_buf().join("peer_db/wallet");
        wallet_config.wallet.p2p.dht = DhtConfig::default_local_test();

        wallet_config.wallet.custom_base_node = Some(format!(
            "{}::/ip4/127.0.0.1/tcp/{}",
            base_node_public_key, base_node_grpc_port
        ));

        let mut builder = runtime::Builder::new_multi_thread();
        let rt = builder.enable_all().build().unwrap();

        let result = run_wallet(rt, &mut wallet_config);
        if let Err(e) = result {
            println!("{:?}", e);
            panic!();
        }
    });

    // make the new wallet able to be referenced by other processes
    let wallet_process = WalletProcess {
        name: wallet_name.clone(),
        port,
        grpc_port,
        handle,
    };
    world.wallets.insert(wallet_name, wallet_process);

    // We need to give it time for the wallet to startup
    // TODO: it would be better to scan the wallet to detect when it has started
    thread::sleep(Duration::from_secs(5));
}
