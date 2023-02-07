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

use std::{
    convert::{Infallible, TryFrom},
    future,
    io,
    time::Duration,
};

use async_trait::async_trait;
use cucumber::{
    gherkin::{Scenario, Step},
    given,
    then,
    when,
    writer,
    WorldInit,
    WriterExt,
};
use indexmap::IndexMap;
use tari_common::initialize_logging;
use tari_common_types::types::PublicKey;
use tari_crypto::tari_utilities::hex::Hex;
use tari_dan_common_types::QuorumDecision;
use tari_dan_core::services::BaseNodeClient;
use tari_engine_types::execution_result::Type;
use tari_template_lib::Hash;
use tari_validator_node::GrpcBaseNodeClient;
use tari_validator_node_cli::versioned_substate_address::VersionedSubstateAddress;
use tari_validator_node_client::types::{GetIdentityResponse, GetTemplateRequest, TemplateRegistrationResponse};
use utils::{
    miner::{mine_blocks, register_miner_process},
    validator_node::spawn_validator_node,
    validator_node_cli,
    wallet::spawn_wallet,
};

use crate::utils::{
    base_node::{get_base_node_client, spawn_base_node, BaseNodeProcess},
    http_server::MockHttpServer,
    logging::create_log_config_file,
    miner::MinerProcess,
    template::{send_template_registration, RegisteredTemplate},
    validator_node::{get_vn_client, ValidatorNodeProcess},
    wallet::WalletProcess,
};

#[derive(Debug, Default, WorldInit)]
pub struct TariWorld {
    base_nodes: IndexMap<String, BaseNodeProcess>,
    wallets: IndexMap<String, WalletProcess>,
    validator_nodes: IndexMap<String, ValidatorNodeProcess>,
    miners: IndexMap<String, MinerProcess>,
    templates: IndexMap<String, RegisteredTemplate>,
    outputs: IndexMap<String, IndexMap<String, VersionedSubstateAddress>>,
    http_server: Option<MockHttpServer>,
    cli_data_dir: Option<String>,
    current_scenario_name: Option<String>,
    commitments: IndexMap<String, Vec<u8>>,
    commitment_ownership_proofs: IndexMap<String, Vec<u8>>,
    rangeproofs: IndexMap<String, Vec<u8>>,
    addresses: IndexMap<String, String>,
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

    pub fn after(&mut self, _scenario: &Scenario) {
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
        self.miners.clear();
    }
}

#[async_trait(?Send)]
impl cucumber::World for TariWorld {
    type Error = Infallible;

    async fn new() -> Result<Self, Self::Error> {
        Ok(Self::default())
    }
}

#[tokio::main]
async fn main() {
    let log_path = create_log_config_file();
    initialize_logging(log_path.as_path(), include_str!("./log4rs/cucumber.yml")).unwrap();

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
        .after(move |_feature, _rule, scenario, maybe_world| {
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

#[given(expr = "a validator node {word} connected to base node {word} and wallet {word}")]
async fn start_validator_node(world: &mut TariWorld, vn_name: String, bn_name: String, wallet_name: String) {
    spawn_validator_node(world, vn_name, bn_name, wallet_name).await;
}

#[when(expr = "validator node {word} sends a registration transaction")]
async fn send_vn_registration(world: &mut TariWorld, vn_name: String) {
    let jrpc_port = world.validator_nodes.get(&vn_name).unwrap().json_rpc_port;
    let mut client = get_vn_client(jrpc_port).await;

    let _resp = client.register_validator_node().await.unwrap();

    // give it some time for the registration tx to be processed by the wallet and base node
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
    validator_node_cli::create_component(world, component_name, template_name, vn_name, function_call, vec![], 1).await;

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

#[when(expr = "I create a DAN wallet")]
async fn create_dan_wallet(world: &mut TariWorld) {
    validator_node_cli::create_dan_wallet(world).await;
}

#[when(expr = "I create an account {word} on {word}")]
async fn create_account(world: &mut TariWorld, account_name: String, vn_name: String) {
    validator_node_cli::create_account(world, account_name, vn_name).await;
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

fn wrap_manifest_in_main(world: &TariWorld, contents: &str) -> String {
    // define all templates
    let template_defs = world.templates.iter().fold(String::new(), |acc, (name, template)| {
        format!("{}\nuse template_{} as {};", acc, template.address, name)
    });
    format!("{} fn main() {{ {} }}", template_defs, contents)
}

#[when(expr = "I wait {int} seconds")]
async fn wait_seconds(_world: &mut TariWorld, seconds: u64) {
    tokio::time::sleep(Duration::from_secs(seconds)).await;
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
            "Validator node \"{}\": json rpc port \"{}\", http ui port \"{}\", temp dir path \"{}\"",
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

    eprintln!();
    eprintln!("======================================");
    eprintln!();
}
