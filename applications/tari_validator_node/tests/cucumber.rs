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

mod steps;
mod utils;
use std::{collections::HashMap, convert::TryFrom, future, io, str::FromStr, time::Duration};

use cucumber::{
    gherkin::{Scenario, Step},
    given,
    then,
    when,
    writer,
    World,
    WriterExt,
};
use indexmap::IndexMap;
use tari_common::initialize_logging;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_comms::multiaddr::Multiaddr;
use tari_crypto::{
    ristretto::{RistrettoComSig, RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::hex::Hex,
};
use tari_dan_app_utilities::base_node_client::GrpcBaseNodeClient;
use tari_dan_common_types::QuorumDecision;
use tari_dan_core::services::BaseNodeClient;
use tari_dan_engine::abi::Type;
use tari_template_lib::Hash;
use tari_validator_node_cli::versioned_substate_address::VersionedSubstateAddress;
use tari_validator_node_client::types::{
    AddPeerRequest,
    GetIdentityResponse,
    GetRecentTransactionsRequest,
    GetTemplateRequest,
    GetTransactionResultRequest,
    TemplateRegistrationResponse,
};
use utils::{
    indexer::spawn_indexer,
    miner::{mine_blocks, register_miner_process},
    validator_node::spawn_validator_node,
    validator_node_cli,
    wallet::spawn_wallet,
    wallet_daemon_cli,
};

use crate::utils::{
    base_node::{get_base_node_client, spawn_base_node, BaseNodeProcess},
    http_server::MockHttpServer,
    indexer::IndexerProcess,
    logging::{create_log_config_file, get_base_dir},
    miner::MinerProcess,
    template::{send_template_registration, RegisteredTemplate},
    validator_node::{get_vn_client, ValidatorNodeProcess},
    wallet::WalletProcess,
    wallet_daemon::{spawn_wallet_daemon, DanWalletDaemonProcess},
};

#[derive(Debug, Default, cucumber::World)]
pub struct TariWorld {
    pub base_nodes: IndexMap<String, BaseNodeProcess>,
    pub wallets: IndexMap<String, WalletProcess>,
    pub validator_nodes: IndexMap<String, ValidatorNodeProcess>,
    pub indexers: IndexMap<String, IndexerProcess>,
    pub vn_seeds: IndexMap<String, ValidatorNodeProcess>,
    pub miners: IndexMap<String, MinerProcess>,
    pub templates: IndexMap<String, RegisteredTemplate>,
    pub outputs: IndexMap<String, IndexMap<String, VersionedSubstateAddress>>,
    pub http_server: Option<MockHttpServer>,
    pub cli_data_dir: Option<String>,
    pub current_scenario_name: Option<String>,
    pub commitments: IndexMap<String, Vec<u8>>,
    pub commitment_ownership_proofs: IndexMap<String, RistrettoComSig>,
    pub rangeproofs: IndexMap<String, Vec<u8>>,
    pub addresses: IndexMap<String, String>,
    pub num_databases_saved: usize,
    pub account_public_keys: IndexMap<String, (RistrettoSecretKey, PublicKey)>,
    pub claim_public_keys: IndexMap<String, PublicKey>,
    pub wallet_daemons: IndexMap<String, DanWalletDaemonProcess>,
    pub wallet_daemon_outputs: IndexMap<String, IndexMap<String, VersionedSubstateAddress>>,
}

impl TariWorld {
    pub fn get_miner(&self, name: &str) -> &MinerProcess {
        self.miners
            .get(name)
            .unwrap_or_else(|| panic!("Miner {} not found", name))
    }

    pub fn get_wallet(&self, name: &str) -> &WalletProcess {
        self.wallets
            .get(name)
            .unwrap_or_else(|| panic!("Wallet {} not found", name))
    }

    pub fn get_base_node(&self, name: &str) -> &BaseNodeProcess {
        self.base_nodes
            .get(name)
            .unwrap_or_else(|| panic!("Base node {} not found", name))
    }

    pub fn get_account_component_address(&self, name: &str) -> Option<String> {
        let all_components = self
            .outputs
            .get(name)
            .unwrap_or_else(|| panic!("Account component address {} not found", name));
        all_components.get("components/Account").map(|a| a.address.to_string())
    }

    pub fn after(&mut self, _scenario: &Scenario) {
        for (name, mut p) in self.indexers.drain(..) {
            println!("Shutting down indexer {}", name);
            p.shutdown.trigger();
        }

        for (name, mut p) in self.validator_nodes.drain(..) {
            println!("Shutting down validator node {}", name);
            p.shutdown.trigger();
        }

        for (name, mut p) in self.wallets.drain(..) {
            println!("Shutting down wallet {}", name);
            p.shutdown.trigger();
        }
        for (name, mut p) in self.base_nodes.drain(..) {
            println!("Shutting down base node {}", name);
            // You have explicitly trigger the shutdown now because of the change to use Arc/Mutex in tari_shutdown
            p.shutdown.trigger();
        }
        for (name, mut p) in self.wallet_daemons.drain(..) {
            println!("Shutting down wallet daemon {}", name);
            // You have explicitly trigger the shutdown now because of the change to use Arc/Mutex in tari_shutdown
            p.shutdown.trigger();
        }
        println!("Removing validator node outputs");
        self.outputs.clear();
        println!("Removing wallet daemon outputs");
        self.wallet_daemon_outputs.clear();
        self.miners.clear();
    }
}

#[tokio::main]
async fn main() {
    let log_path = create_log_config_file();
    let base_path = get_base_dir();
    initialize_logging(log_path.as_path(), &base_path, include_str!("./log4rs/cucumber.yml")).unwrap();

    TariWorld::cucumber()
        .max_concurrent_scenarios(1)
        .with_writer(
            // following config needed to use eprint statements in the tests
            writer::Basic::raw(io::stdout(), writer::Coloring::Auto, 0)
                .summarized()
                .assert_normalized(),
        )
        .before(move |_feature, _rule, scenario, world| {
            world.current_scenario_name = Some(scenario.name.clone());
            Box::pin(future::ready(()))
        })
        .after(move |_feature, _rule, scenario, _finished, maybe_world| {
            if let Some(world) = maybe_world {
                world.after(scenario);
            }
            Box::pin(future::ready(()))
        })
        .fail_on_skipped()
        .run_and_exit("tests/features/")
        .await;
}

#[given(expr = "a base node {word}")]
async fn start_base_node(world: &mut TariWorld, bn_name: String) {
    spawn_base_node(world, bn_name).await;
}

#[given(expr = "a seed validator node {word} connected to base node {word} and wallet {word}")]
async fn start_seed_validator_node(world: &mut TariWorld, seed_vn_name: String, bn_name: String, wallet_name: String) {
    spawn_validator_node(world, seed_vn_name.clone(), bn_name, wallet_name, true).await;
}

#[given(expr = "{int} validator nodes connected to base node {word} and wallet {word}")]
async fn start_multiple_validator_nodes(world: &mut TariWorld, num_nodes: u64, bn_name: String, wallet_name: String) {
    for i in 1..=num_nodes {
        let vn_name = format!("VAL_{i}");
        spawn_validator_node(world, vn_name, bn_name.clone(), wallet_name.clone(), false).await;
    }
}

#[given(expr = "validator {word} nodes connect to all other validators")]
async fn given_validator_connects_to_other_vns(world: &mut TariWorld, vn: String) {
    let details = world
        .validator_nodes
        .values()
        .map(|vn| {
            (
                PublicKey::from_hex(&vn.public_key).unwrap(),
                Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", vn.port)).unwrap(),
            )
        })
        .collect::<Vec<_>>();

    let vn = world.validator_nodes.get_mut(&vn).unwrap();
    let mut cli = vn.create_client().await;
    let this_pk = RistrettoPublicKey::from_hex(&vn.public_key).unwrap();
    for (pk, addr) in details.iter().cloned() {
        if pk == this_pk {
            continue;
        }
        cli.add_peer(AddPeerRequest {
            public_key: pk,
            addresses: vec![addr],
            wait_for_dial: true,
        })
        .await
        .unwrap();
    }
}

#[given(expr = "a validator node {word} connected to base node {word} and wallet {word}")]
async fn start_validator_node(world: &mut TariWorld, vn_name: String, bn_name: String, wallet_name: String) {
    spawn_validator_node(world, vn_name, bn_name, wallet_name, false).await;
}

#[when(expr = "I stop validator node {word}")]
async fn stop_validator_node(world: &mut TariWorld, vn_name: String) {
    let vn_ps = world.validator_nodes.get_mut(&vn_name).unwrap();
    vn_ps.stop();
}

#[given(expr = "a wallet daemon {word} connected to indexer {word}")]
async fn start_wallet_daemon(world: &mut TariWorld, wallet_daemon_name: String, indexer_name: String) {
    spawn_wallet_daemon(world, wallet_daemon_name, indexer_name).await;
}

#[when(expr = "I stop wallet daemon {word}")]
async fn stop_wallet_daemon(world: &mut TariWorld, wallet_daemon_name: String) {
    let walletd_ps = world.wallet_daemons.get_mut(&wallet_daemon_name).unwrap();
    walletd_ps.stop();
}

#[when(expr = "validator node {word} sends a registration transaction")]
async fn send_vn_registration(world: &mut TariWorld, vn_name: String) {
    let jrpc_port = world.validator_nodes.get(&vn_name).unwrap().json_rpc_port;
    let mut client = get_vn_client(jrpc_port).await;

    let _resp = client.register_validator_node().await.unwrap();

    // give it some time for the registration tx to be processed by the wallet and base node
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = "all validator nodes send registration transactions")]
async fn all_vns_send_registration(world: &mut TariWorld) {
    for vn_ps in world.validator_nodes.values() {
        let jrpc_port = vn_ps.json_rpc_port;
        let mut client = get_vn_client(jrpc_port).await;
        let _resp = client.register_validator_node().await.unwrap();
    }

    eprintln!("WAITING AND GETS HERE");
    // give it some time for the registration tx to be processed by the wallet and the base node
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = "validator node {word} registers the template \"{word}\"")]
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

#[then(expr = "all validator nodes are listed as registered")]
async fn assert_all_vns_are_registered(world: &mut TariWorld) {
    for vn_ps in world.validator_nodes.values() {
        // create a base node client
        let base_node_grpc_port = vn_ps.base_node_grpc_port;
        let mut base_node_client: GrpcBaseNodeClient = get_base_node_client(base_node_grpc_port).await;

        // get the list of registered vns from the base node
        let height = base_node_client.get_tip_info().await.unwrap().height_of_longest_chain;
        let vns = base_node_client.get_validator_nodes(height).await.unwrap();
        assert!(!vns.is_empty());

        // retrieve the VN's public key
        let jrpc_port = vn_ps.json_rpc_port;
        let mut client = get_vn_client(jrpc_port).await;
        let identity: GetIdentityResponse = client.get_identity().await.unwrap();
        let public_key: PublicKey = PublicKey::from_hex(&identity.public_key).unwrap();

        // check that the vn's public key is in the list of registered vns
        assert!(vns.iter().any(|vn| vn.public_key == public_key));
    }
}

#[then(expr = "the validator node {word} is listed as registered")]
async fn assert_vn_is_registered(world: &mut TariWorld, vn_name: String) {
    // create a base node client
    let base_node_grpc_port = world.validator_nodes.get(&vn_name).unwrap().base_node_grpc_port;
    let mut base_node_client: GrpcBaseNodeClient = get_base_node_client(base_node_grpc_port).await;

    // get the list of registered vns from the base node
    let height = base_node_client.get_tip_info().await.unwrap().height_of_longest_chain;
    let vns = base_node_client.get_validator_nodes(height).await.unwrap();
    assert!(!vns.is_empty());

    // retrieve the VN's public key
    let jrpc_port = world.validator_nodes.get(&vn_name).unwrap().json_rpc_port;
    let mut client = get_vn_client(jrpc_port).await;
    let identity: GetIdentityResponse = client.get_identity().await.unwrap();
    let public_key: PublicKey = PublicKey::from_hex(&identity.public_key).unwrap();

    // check that the vn's public key is in the list of registered vns
    assert!(vns.iter().any(|vn| vn.public_key == public_key));
}

#[then(expr = "the template \"{word}\" is listed as registered by the validator node {word}")]
async fn assert_template_is_registered(world: &mut TariWorld, template_name: String, vn_name: String) {
    // give it some time for the template tx to be picked up by the VNs
    tokio::time::sleep(Duration::from_secs(4)).await;

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

#[then(expr = "the template \"{word}\" is listed as registered by all validator nodes")]
async fn assert_template_is_registered_by_all(world: &mut TariWorld, template_name: String) {
    // give it some time for the template tx to be picked up by the VNs
    tokio::time::sleep(Duration::from_secs(4)).await;

    // retrieve the template address
    let template_address = world.templates.get(&template_name).unwrap().address;

    // try to get the template for each VN
    for vn_ps in world.validator_nodes.values() {
        let jrpc_port = vn_ps.json_rpc_port;
        let mut client = get_vn_client(jrpc_port).await;
        let req = GetTemplateRequest { template_address };
        let resp = client.get_template(req).await.unwrap();

        // check that the template is indeed in the response
        assert_eq!(resp.registration_metadata.address, template_address);
    }
}

#[when(
    expr = r#"I call function "{word}" on template "{word}" using account {word} to pay fees via wallet daemon {word} with args "{word}" and {int} outputs named "{word}""#
)]
async fn call_template_constructor_via_wallet_daemon(
    world: &mut TariWorld,
    function_call: String,
    template_name: String,
    account_name: String,
    wallet_daemon_name: String,
    args: String,
    num_outputs: u64,
    outputs_name: String,
) {
    let args = args.split(',').map(|a| a.trim().to_string()).collect();
    wallet_daemon_cli::create_component(
        world,
        outputs_name,
        template_name,
        account_name,
        wallet_daemon_name,
        function_call,
        args,
        num_outputs,
    )
    .await;

    // give it some time between transactions
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(
    expr = r#"I call function "{word}" on template "{word}" on {word} with args "{word}" and {int} outputs named "{word}""#
)]
async fn call_template_constructor(
    world: &mut TariWorld,
    function_call: String,
    template_name: String,
    vn_name: String,
    args: String,
    num_outputs: u64,
    outputs_name: String,
) {
    let args = args.split(',').map(|a| a.trim().to_string()).collect();
    validator_node_cli::create_component(
        world,
        outputs_name,
        template_name,
        vn_name,
        function_call,
        args,
        num_outputs,
        vec![],
    )
    .await;

    // give it some time between transactions
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(
    expr = r#"I call function "{word}" on template "{word}" on {word} with args "{word}" and {int} outputs named "{word}" with new resource "{word}""#
)]
async fn call_template_constructor_resource(
    world: &mut TariWorld,
    function_call: String,
    template_name: String,
    vn_name: String,
    args: String,
    num_outputs: u64,
    outputs_name: String,
    new_resource_token: String,
) {
    let args = args.split(',').map(|a| a.trim().to_string()).collect();
    validator_node_cli::create_component(
        world,
        outputs_name,
        template_name,
        vn_name,
        function_call,
        args,
        num_outputs,
        vec![new_resource_token],
    )
    .await;

    // give it some time between transactions
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(
    expr = r#"I call function "{word}" on template "{word}" on {word} with {int} outputs named "{word}" with new resource "{word}""#
)]
async fn call_template_constructor_with_no_args(
    world: &mut TariWorld,
    function_call: String,
    template_name: String,
    vn_name: String,
    num_outputs: u64,
    outputs_name: String,
    new_resource_token_symbol: String,
) {
    validator_node_cli::create_component(
        world,
        outputs_name,
        template_name,
        vn_name,
        function_call,
        vec![],
        num_outputs,
        vec![new_resource_token_symbol],
    )
    .await;

    // give it some time between transactions
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = r#"I create a component {word} of template "{word}" on {word} using "{word}""#)]
async fn call_template_constructor_without_args(
    world: &mut TariWorld,
    component_name: String,
    template_name: String,
    vn_name: String,
    function_call: String,
) {
    validator_node_cli::create_component(
        world,
        component_name,
        template_name,
        vn_name,
        function_call,
        vec![],
        1,
        vec![],
    )
    .await;

    // give it some time between transactions
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = r#"I create a component {word} of template "{word}" on {word} using "{word}" and new resource "{word}"#)]
async fn call_template_constructor_without_args_and_resource(
    world: &mut TariWorld,
    component_name: String,
    template_name: String,
    vn_name: String,
    function_call: String,
    new_resource_token_symbol: String,
) {
    validator_node_cli::create_component(
        world,
        component_name,
        template_name,
        vn_name,
        function_call,
        vec![],
        1,
        vec![new_resource_token_symbol],
    )
    .await;

    // give it some time between transactions
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = r#"I invoke on {word} on component {word} the method call "{word}" with {int} outputs named "{word}""#)]
async fn call_component_method(
    world: &mut TariWorld,
    vn_name: String,
    component_name: String,
    method_call: String,
    num_outputs: u64,
    output_name: String,
) {
    let resp =
        validator_node_cli::call_method(world, vn_name, component_name, output_name, method_call, num_outputs).await;
    assert_eq!(resp.result.unwrap().decision, QuorumDecision::Accept);

    // give it some time between transactions
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(
    expr = r#"I invoke on all validator nodes on component {word} the method call "{word}" with {int} outputs named "{word}""#
)]
async fn call_component_method_on_all_vns(
    world: &mut TariWorld,
    component_name: String,
    method_call: String,
    num_outputs: u64,
    output_name: String,
) {
    let vn_names = world.validator_nodes.iter().map(|(v, _)| v.clone()).collect::<Vec<_>>();
    for vn_name in vn_names {
        let resp = validator_node_cli::call_method(
            world,
            vn_name,
            component_name.clone(),
            output_name.clone(),
            method_call.clone(),
            num_outputs,
        )
        .await;
        assert_eq!(resp.result.unwrap().decision, QuorumDecision::Accept);
    }
    // give it some time between transactions
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(
    expr = "I invoke on {word} on component {word} the method call \"{word}\" with {int} outputs the result is \
            \"{word}\""
)]
async fn call_component_method_and_check_result(
    world: &mut TariWorld,
    vn_name: String,
    component_name: String,
    method_call: String,
    num_outputs: u64,
    expected_result: String,
) {
    let resp = validator_node_cli::call_method(
        world,
        vn_name,
        component_name,
        "dummy_outputs".to_string(),
        method_call,
        num_outputs,
    )
    .await;
    let finalize_result = resp.result.unwrap();
    assert_eq!(finalize_result.decision, QuorumDecision::Accept);

    let results = finalize_result.finalize.execution_results;
    let result = results.first().unwrap();
    match result.return_type {
        Type::U32 => {
            let u32_result: u32 = result.decode().unwrap();
            assert_eq!(u32_result.to_string(), expected_result);
        },
        // TODO: handle other possible return types
        _ => todo!(),
    };

    // give it some time between transactions
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(
    expr = "I invoke on all validator nodes on component {word} the method call \"{word}\" with {int} outputs the \
            result is \"{word}\""
)]
async fn call_component_method_on_all_vns_and_check_result(
    world: &mut TariWorld,
    component_name: String,
    method_call: String,
    num_outputs: u64,
    expected_result: String,
) {
    let vn_names = world.validator_nodes.iter().map(|(v, _)| v.clone()).collect::<Vec<_>>();
    for vn_name in vn_names {
        let resp = validator_node_cli::call_method(
            world,
            vn_name,
            component_name.clone(),
            "dummy_outputs".to_string(),
            method_call.clone(),
            num_outputs,
        )
        .await;
        let finalize_result = resp.result.unwrap();
        assert_eq!(finalize_result.decision, QuorumDecision::Accept);

        let results = finalize_result.finalize.execution_results;
        let result = results.first().unwrap();
        match result.return_type {
            Type::U32 => {
                let u32_result: u32 = result.decode().unwrap();
                assert_eq!(u32_result.to_string(), expected_result);
            },
            // TODO: handle other possible return types
            _ => todo!(),
        };
    }

    // give it some time between transactions
    tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = "I create a DAN wallet")]
async fn create_dan_wallet(world: &mut TariWorld) {
    validator_node_cli::create_dan_wallet(world);
}

#[when(expr = "I create an account {word} on {word}")]
async fn create_account(world: &mut TariWorld, account_name: String, vn_name: String) {
    validator_node_cli::create_account(world, account_name, vn_name).await;
}

#[when(expr = "I create {int} accounts on {word}")]
async fn create_multiple_accounts(world: &mut TariWorld, num_accounts: u64, vn_name: String) {
    for i in 1..=num_accounts {
        let account_name = format!("ACC_{i}");
        validator_node_cli::create_account(world, account_name, vn_name.clone()).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[when(expr = r#"I submit a transaction manifest on {word} with {int} outputs named "{word}""#)]
async fn submit_manifest(world: &mut TariWorld, step: &Step, vn_name: String, num_outputs: u64, output_name: String) {
    let manifest = wrap_manifest_in_main(world, step.docstring.as_ref().expect("manifest code not provided"));
    validator_node_cli::submit_manifest(world, vn_name, output_name, manifest, String::new(), num_outputs).await;
}

#[when(regex = r#"^I submit a transaction manifest on (\w+) with inputs "([^"]+)" and (\d+) outputs? named "(\w+)"$"#)]
async fn submit_manifest_with_inputs(
    world: &mut TariWorld,
    step: &Step,
    vn_name: String,
    inputs: String,
    num_outputs: u64,
    outputs_name: String,
) {
    let manifest = wrap_manifest_in_main(world, step.docstring.as_ref().expect("manifest code not provided"));
    validator_node_cli::submit_manifest(world, vn_name, outputs_name, manifest, inputs, num_outputs).await;
}

#[when(expr = "account {word} reveals {int} burned tokens via wallet daemon {word}")]
async fn reveal_burned_funds(world: &mut TariWorld, account_name: String, amount: u64, wallet_daemon_name: String) {
    wallet_daemon_cli::reveal_burned_funds(world, account_name, amount, wallet_daemon_name).await;
}

#[when(
    regex = r#"^I submit a transaction manifest via wallet daemon (\w+) with inputs "([^"]+)" and (\d+) outputs? named "(\w+)"$"#
)]
async fn submit_transaction_manifest_via_wallet_daemon(
    world: &mut TariWorld,
    step: &Step,
    wallet_daemon_name: String,
    inputs: String,
    num_outputs: u64,
    outputs_name: String,
) {
    let manifest = wrap_manifest_in_main(world, step.docstring.as_ref().expect("manifest code not provided"));
    wallet_daemon_cli::submit_manifest(world, wallet_daemon_name, manifest, inputs, num_outputs, outputs_name).await;
}

#[when(
    regex = r#"^I submit a transaction manifest via wallet daemon (\w+) signed by the key of (\w+) with inputs "([^"]+)" and (\d+) outputs? named "(\w+)"$"#
)]
async fn submit_transaction_manifest_via_wallet_daemon_with_signing_keys(
    world: &mut TariWorld,
    step: &Step,
    wallet_daemon_name: String,
    account_signing_key: String,
    inputs: String,
    num_outputs: u64,
    outputs_name: String,
) {
    let manifest = wrap_manifest_in_main(world, step.docstring.as_ref().expect("manifest code not provided"));
    wallet_daemon_cli::submit_manifest_with_signing_keys(
        world,
        wallet_daemon_name,
        account_signing_key,
        manifest,
        inputs,
        num_outputs,
        outputs_name,
    )
    .await;
}

fn wrap_manifest_in_main(world: &TariWorld, contents: &str) -> String {
    // define all templates
    let template_defs = world.templates.iter().fold(String::new(), |acc, (name, template)| {
        format!("{}\nuse template_{} as {};", acc, template.address, name)
    });
    format!("{} fn main() {{ {} }}", template_defs, contents)
}

#[given(expr = "an indexer {word} connected to base node {word}")]
async fn start_indexer(world: &mut TariWorld, indexer_name: String, bn_name: String) {
    spawn_indexer(world, indexer_name, bn_name).await;
}

#[given(expr = "{word} indexer GraphQL request works")]
async fn works_indexer_graphql(world: &mut TariWorld, indexer_name: String) {
    let indexer: &mut IndexerProcess = world.indexers.get_mut(&indexer_name).unwrap();
    // insert event mock data in the substate manager database
    indexer.insert_event_mock_data().await;
    let mut graphql_client = indexer.get_graphql_indexer_client().await;
    let template_address = [0u8; 32];
    let tx_hash = [0u8; 32];
    let query = format!(
        "{{ getEvent(templateAddress: {:?}, txHash: {:?}) {{ templateAddress, txHash, topic, payload }} }}",
        template_address, tx_hash
    );
    let res = graphql_client
        .send_request::<HashMap<String, Vec<tari_indexer::graphql::model::events::Event>>>(&query, None, None)
        .await
        .unwrap();
    let res = res.get("getEvent").unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].template_address, [0u8; 32]);
    assert_eq!(res[0].tx_hash, [0u8; 32]);
    assert_eq!(res[0].topic, "my_event");
    assert_eq!(res[0].payload, HashMap::from([("my".to_string(), "event".to_string())]));
}

#[when(expr = "the indexer {word} tracks the address {word}")]
async fn track_addresss_in_indexer(world: &mut TariWorld, indexer_name: String, output_ref: String) {
    let indexer = world.indexers.get(&indexer_name).unwrap();
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);
    indexer.add_address(world, output_ref).await;
}

#[then(expr = "the indexer {word} returns version {int} for substate {word}")]
async fn assert_indexer_substate_version(
    world: &mut TariWorld,
    indexer_name: String,
    version: u32,
    output_ref: String,
) {
    let indexer = world.indexers.get(&indexer_name).unwrap();
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);
    let substate = indexer.get_substate(world, output_ref, version).await;
    eprintln!(
        "indexer.get_substate result: {}",
        serde_json::to_string_pretty(&substate).unwrap()
    );
    assert_eq!(substate.version, version);
}

#[then(expr = "the indexer {word} returns {int} non fungibles for resource {word}")]
async fn assert_indexer_non_fungible_list(
    world: &mut TariWorld,
    indexer_name: String,
    count: usize,
    output_ref: String,
) {
    let indexer = world.indexers.get(&indexer_name).unwrap();
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);
    let nfts = indexer.get_non_fungibles(world, output_ref, 0, count as u64).await;
    eprintln!("indexer.get_non_fungibles result: {:?}", nfts);
    assert_eq!(
        nfts.len(),
        count,
        "Unexpected number of NFTs returned. Expected: {}, Actual: {}",
        count,
        nfts.len()
    );
}

#[when(expr = "I wait {int} seconds")]
async fn wait_seconds(_world: &mut TariWorld, seconds: u64) {
    tokio::time::sleep(Duration::from_secs(seconds)).await;
}

#[then(expr = "all transactions succeed on all validator nodes")]
async fn successful_transaction(world: &mut TariWorld) {
    // loop over each validator node to check if transaction
    // was accepted by each
    for vn_ps in world.validator_nodes.values() {
        let jrpc_port = vn_ps.json_rpc_port;
        let mut client = get_vn_client(jrpc_port).await;

        let request = GetRecentTransactionsRequest {};
        let recent_transactions_res = client.get_recent_transactions(request).await.unwrap();

        let recent_transactions = recent_transactions_res.transactions;
        // check that all transactions have succeeded
        for tx in &recent_transactions {
            let payload_id = &tx.payload_id[..];
            assert_eq!(payload_id.len(), 32);

            let hash = FixedHash::try_from(payload_id).unwrap();
            let get_transaction_req = GetTransactionResultRequest { hash };
            let get_transaction_res = client.get_transaction_result(get_transaction_req).await.unwrap();
            let finalized_tx = get_transaction_res.result.unwrap();
            finalized_tx.expect_success();
        }
    }
}

#[when(expr = "I print the cucumber world")]
async fn print_world(world: &mut TariWorld) {
    eprintln!();
    eprintln!("======================================");
    eprintln!("============= TEST STATE =============");
    eprintln!("======================================");
    eprintln!();

    // base nodes
    for (name, node) in world.base_nodes.iter() {
        eprintln!(
            "Base node \"{}\": grpc port \"{}\", temp dir path \"{}\"",
            name, node.grpc_port, node.temp_dir_path
        );
    }

    // wallets
    for (name, node) in world.wallets.iter() {
        eprintln!(
            "Wallet \"{}\": grpc port \"{}\", temp dir path \"{}\"",
            name, node.grpc_port, node.temp_dir_path
        );
    }

    // vns
    for (name, node) in world.validator_nodes.iter() {
        eprintln!(
            "Validator node \"{}\": json rpc port \"{}\", http ui port \"{}\", temp dir path \"{:?}\"",
            name, node.json_rpc_port, node.http_ui_port, node.temp_dir_path
        );
    }

    // indexes
    for (name, node) in world.indexers.iter() {
        eprintln!(
            "Indexer \"{}\": json rpc port \"{}\", http ui port  \"{}\", temp dir path \"{}\"",
            name, node.json_rpc_port, node.http_ui_port, node.temp_dir_path
        );
    }

    // templates
    for (name, template) in world.templates.iter() {
        eprintln!("Template \"{}\" with address \"{}\"", name, template.address);
    }

    // templates
    for (name, outputs) in world.outputs.iter() {
        eprintln!("Outputs \"{}\"", name);
        for (name, addr) in outputs {
            eprintln!("  - {}: {}", name, addr);
        }
    }

    // wallet daemons
    for (name, daemon) in world.wallet_daemons.iter() {
        eprintln!("Wallet daemons \"{}\"", name);
        eprintln!("  - {}: {}", name, daemon.name);
    }

    // wallet daemon substate addresses
    for (name, outputs) in world.wallet_daemon_outputs.iter() {
        eprintln!("Outputs \"{}\"", name);
        for (name, addr) in outputs {
            eprintln!("  - {}: {}", name, addr);
        }
    }

    eprintln!();
    eprintln!("======================================");
    eprintln!();
}

#[when(expr = "I save the {word} database of {word}")]
async fn when_i_save_the_database(world: &mut TariWorld, database_name: String, validator_name: String) {
    let validator = world
        .validator_nodes
        .get(&validator_name)
        .expect("validator node not found");
    validator
        .save_database(
            database_name,
            get_base_dir()
                .join(
                    world
                        .current_scenario_name
                        .as_ref()
                        .unwrap_or(&"unknown_step".to_string()),
                )
                .join(format!("save_no_{}", world.num_databases_saved))
                .as_path(),
        )
        .await;
    world.num_databases_saved += 1;
}
