use std::convert::Infallible;

use async_trait::async_trait;
use cucumber::{given, WorldInit};
use tari_common::configuration::CommonConfig;
use tari_p2p::{Network, PeerSeedsConfig};
use tari_validator_node::{run_node, ApplicationConfig, ValidatorNodeConfig};
use tempfile::tempdir;

// `World` is your shared, likely mutable state.
// Cucumber constructs it via `Default::default()` for each scenario.
#[derive(Debug, Default, WorldInit)]
pub struct TariWorld {
    // TODO: add VNS, base nodes, wallets, etc
}

#[async_trait(?Send)]
impl cucumber::World for TariWorld {
    type Error = Infallible;

    async fn new() -> Result<Self, Self::Error> {
        Ok(Self {})
    }
}

#[given(expr = "a validator node {word}")]
async fn start_validator_node(_world: &mut TariWorld, _name: String) {
    // let config_path = PathBuf::from("config/config.toml");
    // let config = load_configuration(config_path, true, &cli)?;
    // let app_config = ApplicationConfig::load_from(&config);
    let mut config = ApplicationConfig {
        common: CommonConfig::default(),
        validator_node: ValidatorNodeConfig::default(),
        peer_seeds: PeerSeedsConfig::default(),
        network: Network::LocalNet,
    };

    // temporal folder for the VN files (e.g. sqlite file)
    let temp_dir = tempdir().unwrap();
    println!("Using temp_dir: {}", temp_dir.path().display());
    config.validator_node.data_dir = temp_dir.path().to_path_buf();
    config.validator_node.shard_key_file = temp_dir.path().join("shard_key.json");

    // TODO: create a new one in a temp folder
    config.validator_node.identity_file = temp_dir.path().join("validator_node_id.json");

    // TODO: use a spawned base node instead of a real one
    config.validator_node.base_node_grpc_address = "127.0.0.1:18152".parse().unwrap();

    // TODO: use a spawned wallet instead of a real one
    config.validator_node.wallet_grpc_address = "127.0.0.1:18153".parse().unwrap();

    let _result = run_node(&config).await;
}

// This runs before everything else, so you can setup things here.
#[tokio::main]
async fn main() {
    // You may choose any executor you like (`tokio`, `async-std`, etc.).
    // You may even have an `async` main, it doesn't matter. The point is that
    // Cucumber is composable. :)
    futures::executor::block_on(TariWorld::run("tests/features/"));
}
