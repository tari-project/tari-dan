use std::{str::FromStr, thread, time::Duration};

use tari_common::configuration::CommonConfig;
use tari_comms::multiaddr::Multiaddr;
use tari_comms_dht::DhtConfig;
use tari_console_wallet::run_wallet;
use tari_p2p::{auto_update::AutoUpdateConfig, Network, PeerSeedsConfig};
use tari_wallet::WalletConfig;
use tempfile::tempdir;
use tokio::runtime;

pub fn spawn_wallet() {
    thread::spawn(move || {
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
        wallet_config.wallet.use_libtor = true;
        wallet_config.wallet.grpc_enabled = true;
        wallet_config.wallet.grpc_address = Some(Multiaddr::from_str("/ip4/127.0.0.1/tcp/18153").unwrap());
        wallet_config.wallet.data_dir = temp_dir.path().to_path_buf().join("data/wallet");
        wallet_config.wallet.db_file = temp_dir.path().to_path_buf().join("db/console_wallet.db");
        wallet_config.wallet.p2p.datastore_path = temp_dir.path().to_path_buf().join("peer_db/wallet");
        wallet_config.wallet.p2p.dht = DhtConfig::default_local_test();
        wallet_config.wallet.custom_base_node = Some("/ip4/127.0.0.1/tcp/18152".to_string());

        let mut builder = runtime::Builder::new_multi_thread();
        let rt = builder.enable_all().build().unwrap();

        let result = run_wallet(rt, &mut wallet_config);
        if let Err(e) = result {
            println!("{:?}", e);
            panic!();
        }
    });

    // We need to give it time for the wallet to startup
    // TODO: it would be better to scan the wallet to detect when it has started
    thread::sleep(Duration::from_secs(2));
}
