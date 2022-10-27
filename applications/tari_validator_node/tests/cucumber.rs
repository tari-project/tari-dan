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

mod utils;

use std::{
    collections::HashMap,
    convert::{Infallible, TryFrom},
    time::Duration,
};

use async_trait::async_trait;
use cucumber::{given, then, when, WorldInit};
use tari_dan_core::services::BaseNodeClient;
use tari_template_lib::Hash;
use tari_validator_node::GrpcBaseNodeClient;
use tari_validator_node_client::types::{GetIdentityResponse, GetTemplateRequest, TemplateRegistrationResponse};
use utils::{
    miner::{mine_blocks, register_miner_process},
    validator_node::spawn_validator_node,
    wallet::spawn_wallet,
};

use crate::utils::{
    base_node::{get_base_node_client, spawn_base_node, BaseNodeProcess},
    http_server::MockHttpServer,
    miner::MinerProcess,
    template::{send_template_registration, RegisteredTemplate},
    validator_node::{get_vn_client, ValidatorNodeProcess},
    wallet::WalletProcess,
};

#[derive(Debug, Default, WorldInit)]
pub struct TariWorld {
    base_nodes: HashMap<String, BaseNodeProcess>,
    wallets: HashMap<String, WalletProcess>,
    validator_nodes: HashMap<String, ValidatorNodeProcess>,
    miners: HashMap<String, MinerProcess>,
    templates: HashMap<String, RegisteredTemplate>,
    http_server: Option<MockHttpServer>,
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
            templates: HashMap::new(),
            http_server: None,
        })
    }
}

#[given(expr = "a base node {word}")]
async fn start_base_node(world: &mut TariWorld, bn_name: String) {
    spawn_base_node(world, bn_name).await;
}

#[given(expr = "a wallet {word} connected to base node {word}")]
async fn start_wallet(world: &mut TariWorld, wallet_name: String, bn_name: String) {
    spawn_wallet(world, wallet_name, bn_name).await;
}

#[given(expr = "a miner {word} connected to base node {word} and wallet {word}")]
async fn create_miner(world: &mut TariWorld, miner_name: String, bn_name: String, wallet_name: String) {
    register_miner_process(world, miner_name, bn_name, wallet_name);
}

#[given(expr = "a validator node {word} connected to base node {word} and wallet {word}")]
async fn start_validator_node(world: &mut TariWorld, vn_name: String, bn_name: String, wallet_name: String) {
    spawn_validator_node(world, vn_name, bn_name, wallet_name).await;
}

#[when(expr = "miner {word} mines {int} new blocks")]
async fn run_miner(world: &mut TariWorld, miner_name: String, num_blocks: u64) {
    mine_blocks(world, miner_name, num_blocks).await;
}

#[when(expr = "validator node {word} sends a registration transaction")]
async fn send_vn_registration(world: &mut TariWorld, vn_name: String) {
    let jrpc_port = world.validator_nodes.get(&vn_name).unwrap().json_rpc_port;
    let mut client = get_vn_client(jrpc_port).await;

    let _resp = client.register_validator_node().await.unwrap();

    // give it some time for the registration tx to be processed by the wallet and base node
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = "the validator node {word} registers the template \"{word}\"")]
async fn register_template(world: &mut TariWorld, vn_name: String, template_name: String) {
    let resp: TemplateRegistrationResponse = send_template_registration(world, template_name.clone(), vn_name).await;
    assert!(resp.transaction_id != 0);
    assert!(!resp.template_address.is_empty());

    // store the template address for future reference
    let registered_template = RegisteredTemplate {
        name: template_name.clone(),
        address: Hash::try_from(resp.template_address.as_slice()).unwrap(),
    };
    world.templates.insert(template_name, registered_template);

    // give it some time for the registration tx to be processed by the wallet and base node
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[then(expr = "the validator node {word} is listed as registered")]
async fn assert_vn_is_registered(world: &mut TariWorld, vn_name: String) {
    let base_node_grpc_port = world.validator_nodes.get(&vn_name).unwrap().base_node_grpc_port;
    let mut base_node_client: GrpcBaseNodeClient = get_base_node_client(base_node_grpc_port).await;

    let height = base_node_client.get_tip_info().await.unwrap().height_of_longest_chain;
    let vns = base_node_client.get_validator_nodes(height).await.unwrap();
    // FIXME: the base node returns an empty list of registered vns, but the registration tx is actually in a block
    assert!(!vns.is_empty());

    let registered_vn = &vns[0];
    let jrpc_port = world.validator_nodes.get(&vn_name).unwrap().json_rpc_port;
    let mut client = get_vn_client(jrpc_port).await;
    let identity: GetIdentityResponse = client.get_identity().await.unwrap();
    assert_eq!(identity.public_key, registered_vn.public_key.to_string());
}

#[then(expr = "the template \"{word}\" is listed as registered by the validator node {word}")]
async fn assert_template_is_registered(world: &mut TariWorld, template_name: String, vn_name: String) {
    // retrieve the template address
    let template_address = world.templates.get(&template_name).unwrap().address;

    // try to get the template from the VN
    let jrpc_port = world.validator_nodes.get(&vn_name).unwrap().json_rpc_port;
    let mut client = get_vn_client(jrpc_port).await;
    let req = GetTemplateRequest { template_address };
    let resp = client.get_template(req).await.unwrap();

    // check that the template is indeed in the response
    assert_eq!(resp.registration_metadata.address, template_address);
}

#[then(expr = "the validator node {word} returns a valid identity")]
async fn assert_valid_vn_identity(world: &mut TariWorld, vn_name: String) {
    let jrpc_port = world.validator_nodes.get(&vn_name).unwrap().json_rpc_port;

    // send the JSON RPC "get_identity" request to the VN
    let mut client = get_vn_client(jrpc_port).await;
    let resp = client.get_identity().await.unwrap();

    println!("VN identity response: {:?}", resp);
}

#[tokio::main]
async fn main() {
    // env_logger::init();
    TariWorld::run("tests/features/").await;
}
