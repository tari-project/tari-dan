use std::thread;

use tari_app_utilities::common_cli_args::CommonCliArgs;
use tari_miner::{cli::Cli, config::MinerConfig};
use tempfile::tempdir;
use tokio::runtime;

use crate::TariWorld;

#[derive(Debug)]
pub struct MinerProcess {
    pub name: String,
    pub base_node_name: String,
    pub wallet_name: String,
}

pub fn register_miner_process(world: &mut TariWorld, miner_name: String, base_node_name: String, wallet_name: String) {
    let miner = MinerProcess {
        name: miner_name.clone(),
        base_node_name,
        wallet_name,
    };
    world.miners.insert(miner_name, miner);
}

pub async fn mine_blocks(world: &mut TariWorld, miner_name: String, num_blocks: u64) {
    let miner = world.miners.get(&miner_name).unwrap();
    let base_node_grpc_port = world.base_nodes.get(&miner.base_node_name).unwrap().grpc_port;
    let wallet_grpc_port = world.wallets.get(&miner.wallet_name).unwrap().grpc_port;

    let config = MinerConfig {
        base_node_grpc_address: format!("/ip4/127.0.0.1/tcp/{}", base_node_grpc_port).parse().unwrap(),
        wallet_grpc_address: format!("/ip4/127.0.0.1/tcp/{}", wallet_grpc_port).parse().unwrap(),
        num_mining_threads: 1,
        ..Default::default()
    };

    let temp_dir = tempdir().unwrap();
    println!("Using miner temp_dir: {}", temp_dir.path().display());
    let data_dir = temp_dir.into_path();
    let data_dir_str = data_dir.clone().into_os_string().into_string().unwrap();
    let mut config_path = data_dir;
    config_path.push("config.toml");

    let cli = Cli {
        common: CommonCliArgs {
            base_path: data_dir_str,
            config: config_path.into_os_string().into_string().unwrap(),
            log_config: None,
            log_level: None,
            config_property_overrides: vec![],
        },
        mine_until_height: Some(1),
        miner_max_blocks: Some(num_blocks),
        miner_min_diff: Some(0),
        miner_max_diff: Some(100000),
    };

    let handle = thread::spawn(move || async { 
        tari_miner::run_miner_with_cli(config, cli).await
    });
    
    // we block test execution until the blocks have been mined
    handle.join().unwrap().await.unwrap();
}
