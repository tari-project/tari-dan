mod utils;

use std::convert::Infallible;

use async_trait::async_trait;
use cucumber::{given, then, WorldInit};
use utils::validator_node::spawn_validator_node;

use crate::utils::{base_node::spawn_base_node, validator_node::send_vn_json_rpc_request};

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

#[given(expr = "a base node {word}")]
async fn start_base_node(_world: &mut TariWorld, _bn_name: String) {
    // TODO: pass grpc port param
    spawn_base_node();
}

#[given(expr = "a validator node {word} connected to base node {word}")]
async fn start_validator_node(_world: &mut TariWorld, _vn_name: String, _bn_name: String) {
    // TODO: pass base node grpc port param
    spawn_validator_node();
}

#[then(expr = "the validator node {word} returns a valid identity")]
async fn assert_valid_vn_identity(_world: &mut TariWorld, _name: String) -> Result<(), String> {
    // TODO: retrieve the VN from the world by name

    // send the JSON RPC "get_identity" request to the VN
    let body: Vec<String> = vec![];
    let resp = send_vn_json_rpc_request(18145, "get_identity".to_string(), body).await;
    assert_eq!(resp.status(), 200);

    // TODO: assert that the body format is correct with the identity
    println!("{}", resp.text().await.unwrap());

    Ok(())
}

#[tokio::main]
async fn main() {
    futures::executor::block_on(TariWorld::run("tests/features/"));
}
