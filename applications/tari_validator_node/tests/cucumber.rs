mod utils;

use std::{collections::HashMap, convert::Infallible};

use async_trait::async_trait;
use cucumber::{given, then, when, WorldInit};
use utils::{
    miner::{mine_blocks, register_miner_process},
    validator_node::spawn_validator_node,
    wallet::spawn_wallet,
};

use crate::utils::{
    base_node::{spawn_base_node, BaseNodeProcess},
    miner::MinerProcess,
    validator_node::{send_vn_json_rpc_request, ValidatorNodeProcess},
    wallet::WalletProcess,
};

#[derive(Debug, Default, WorldInit)]
pub struct TariWorld {
    base_nodes: HashMap<String, BaseNodeProcess>,
    wallets: HashMap<String, WalletProcess>,
    validator_nodes: HashMap<String, ValidatorNodeProcess>,
    miners: HashMap<String, MinerProcess>,
}

#[async_trait(?Send)]
impl cucumber::World for TariWorld {
    type Error = Infallible;

    async fn new() -> Result<Self, Self::Error> {
        Ok(Self {
            base_nodes: HashMap::new(),
            wallets: HashMap::new(),
            validator_nodes: HashMap::new(),
            miners: HashMap::new(),
        })
    }
}

#[given(expr = "a base node {word}")]
async fn start_base_node(world: &mut TariWorld, bn_name: String) {
    spawn_base_node(world, bn_name);
}

#[given(expr = "a wallet {word} connected to base node {word}")]
async fn start_wallet(world: &mut TariWorld, wallet_name: String, bn_name: String) {
    spawn_wallet(world, wallet_name, bn_name);
}

#[given(expr = "a miner {word} connected to base node {word} and wallet {word}")]
async fn create_miner(world: &mut TariWorld, miner_name: String, bn_name: String, wallet_name: String) {
    register_miner_process(world, miner_name, bn_name, wallet_name);
}

#[given(expr = "a validator node {word} connected to base node {word} and wallet {word}")]
async fn start_validator_node(world: &mut TariWorld, vn_name: String, bn_name: String, wallet_name: String) {
    spawn_validator_node(world, vn_name, bn_name, wallet_name);
}

#[when(expr = "miner {word} mines {int} new blocks")]
async fn run_miner(world: &mut TariWorld, miner_name: String, num_blocks: u64) {
    mine_blocks(world, miner_name, num_blocks).await;
}

#[then(expr = "the validator node {word} returns a valid identity")]
async fn assert_valid_vn_identity(_world: &mut TariWorld, _name: String) -> Result<(), String> {
    // TODO: retrieve the VN from the world by name

    // send the JSON RPC "get_identity" request to the VN
    let body: Vec<String> = vec![];
    let resp = send_vn_json_rpc_request(18145, "get_identity".to_string(), body).await;
    assert_eq!(resp.status(), 200);

    // TODO: assert that the body format is correct with the identity
    println!("VN identity response: {:?}", resp.text().await.unwrap());

    Ok(())
}

#[tokio::main]
async fn main() {
    futures::executor::block_on(TariWorld::run("tests/features/"));
}
