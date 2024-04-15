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
use std::{fs, future, io, panic, str::FromStr, time::Duration};

use cucumber::{gherkin::Step, given, then, when, writer, writer::Verbosity, World, WriterExt};
use integration_tests::{
    http_server::{spawn_template_http_server, MockHttpServer},
    logging::{create_log_config_file, get_base_dir},
    miner::{mine_blocks, register_miner_process},
    validator_node_cli,
    wallet::spawn_wallet,
    wallet_daemon::spawn_wallet_daemon,
    wallet_daemon_cli,
    TariWorld,
};
use libp2p::Multiaddr;
use tari_common::initialize_logging;
use tari_dan_engine::abi::Type;
use tari_dan_storage::consensus_models::QuorumDecision;
use tari_shutdown::Shutdown;
use tari_validator_node_client::types::{AddPeerRequest, GetRecentTransactionsRequest, GetTransactionResultRequest};

const LOG_TARGET: &str = "cucumber";

#[tokio::main]
async fn main() {
    let log_path = create_log_config_file();
    let base_path = get_base_dir();
    initialize_logging(log_path.as_path(), &base_path, include_str!("./log4rs/cucumber.yml")).unwrap();

    // Start the mock server that continues to run for the duration of the tests
    let mut shutdown = Shutdown::new();
    let mock_port = spawn_template_http_server(shutdown.to_signal()).await;

    let file = fs::File::create("cucumber-output-junit.xml").unwrap();
    TariWorld::cucumber()
        .max_concurrent_scenarios(1)
        .with_writer(writer::Tee::new(
            writer::JUnit::new(file, Verbosity::ShowWorldAndDocString).normalized(),
            // following config needed to use eprint statements in the tests
            writer::Basic::raw(io::stdout(), writer::Coloring::Auto, Verbosity::ShowWorldAndDocString)
                .normalized()
                .summarized(),
        ))
        .before(move |_feature, _rule, scenario, world| {
            log::info!(target: LOG_TARGET, "\n\n\n");
            log::info!(target: LOG_TARGET, "-------------------------------------------------------");
            log::info!(target: LOG_TARGET, "------------- SCENARIO: {} -------------", scenario.name);
            log::info!(target: LOG_TARGET, "-------------------------------------------------------");
            log::info!(target: LOG_TARGET, "\n\n\n");
            world.current_scenario_name = Some(scenario.name.clone());
            world.fees_enabled = true;
            Box::pin(async move {
                // Each scenario gets a mock connection. As each connection is dropped after the scenario, all the mock
                // urls are deregistered
                world.http_server = Some(MockHttpServer::connect(mock_port).await);
            })
        })
        .after(move |_feature, _rule, scenario, _finished, maybe_world| {
            if let Some(world) = maybe_world {
                world.after(scenario);
            }
            Box::pin(future::ready(()))
        })
        .fail_on_skipped()
        .filter_run("tests/features/", |_, _, sc| !sc.tags.iter().any(|t| t == "ignore"))
        .await;

    shutdown.trigger();
}

#[given(expr = "fees are disabled")]
async fn fees_are_enabled(world: &mut TariWorld) {
    world.fees_enabled = false;
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

#[when(
    expr = r#"I call function "{word}" on template "{word}" using account {word} to pay fees via wallet daemon {word} with args "{word}" named "{word}""#
)]
async fn call_template_constructor_via_wallet_daemon(
    world: &mut TariWorld,
    function_call: String,
    template_name: String,
    account_name: String,
    wallet_daemon_name: String,
    args: String,
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
        None,
        None,
    )
    .await;

    // give it some time between transactions
    // tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = r#"I call function "{word}" on template "{word}" on {word} with args "{word}" named "{word}""#)]
async fn call_template_constructor(
    world: &mut TariWorld,
    function_call: String,
    template_name: String,
    vn_name: String,
    args: String,
    outputs_name: String,
) {
    let args = args.split(',').map(|a| a.trim().to_string()).collect();
    validator_node_cli::create_component(world, outputs_name, template_name, vn_name, function_call, args).await;

    // give it some time between transactions
    // tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = r#"I call function "{word}" on template "{word}" on {word} named "{word}""#)]
async fn call_template_constructor_with_no_args(
    world: &mut TariWorld,
    function_call: String,
    template_name: String,
    vn_name: String,
    outputs_name: String,
) {
    validator_node_cli::create_component(world, outputs_name, template_name, vn_name, function_call, vec![]).await;

    // give it some time between transactions
    // tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = r#"I create a component {word} of template "{word}" on {word} using "{word}""#)]
async fn call_template_constructor_without_args(
    world: &mut TariWorld,
    component_name: String,
    template_name: String,
    vn_name: String,
    function_call: String,
) {
    validator_node_cli::create_component(world, component_name, template_name, vn_name, function_call, vec![]).await;

    // give it some time between transactions
    // tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = r#"I invoke on {word} on component {word} the method call "{word}" named "{word}""#)]
async fn call_component_method(
    world: &mut TariWorld,
    vn_name: String,
    component_name: String,
    method_call: String,
    output_name: String,
) {
    let resp = validator_node_cli::call_method(world, vn_name, component_name, output_name, method_call)
        .await
        .unwrap();
    assert_eq!(resp.dry_run_result.unwrap().decision, QuorumDecision::Accept);

    // give it some time between transactions
    // tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = r#"I invoke on {word} on component {word} the method call "{word}" concurrently {int} times"#)]
async fn call_component_method_concurrently(
    world: &mut TariWorld,
    vn_name: String,
    component_name: String,
    method_call: String,
    times: usize,
) {
    validator_node_cli::concurrent_call_method(world, vn_name, component_name, method_call, times)
        .await
        .unwrap();
}

#[when(
    expr = r#"I invoke on {word} on component {word} the method call "{word}" named "{word}" the result is error {string}"#
)]
async fn call_component_method_must_error(
    world: &mut TariWorld,
    vn_name: String,
    component_name: String,
    method_call: String,
    output_name: String,
    error_msg: String,
) {
    let res = validator_node_cli::call_method(world, vn_name, component_name, output_name, method_call).await;
    if let Err(reject) = res {
        assert!(reject.to_string().contains(&error_msg));
    } else {
        panic!("Expected an error but the call was successful");
    }
}

#[when(expr = r#"I invoke on all validator nodes on component {word} the method call "{word}" named "{word}""#)]
async fn call_component_method_on_all_vns(
    world: &mut TariWorld,
    component_name: String,
    method_call: String,
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
        )
        .await
        .unwrap();
        assert_eq!(resp.dry_run_result.unwrap().decision, QuorumDecision::Accept);
    }
    // give it some time between transactions
    // tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = "I invoke on {word} on component {word} the method call \"{word}\" the result is \"{word}\"")]
async fn call_component_method_and_check_result(
    world: &mut TariWorld,
    vn_name: String,
    component_name: String,
    method_call: String,
    expected_result: String,
) {
    let resp =
        validator_node_cli::call_method(world, vn_name, component_name, "dummy_outputs".to_string(), method_call)
            .await
            .unwrap();
    let finalize_result = resp.dry_run_result.unwrap();
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
    // tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(
    expr = "I invoke on all validator nodes on component {word} the method call \"{word}\" the result is \"{word}\""
)]
async fn call_component_method_on_all_vns_and_check_result(
    world: &mut TariWorld,
    component_name: String,
    method_call: String,
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
        )
        .await
        .unwrap();
        let finalize_result = resp.dry_run_result.unwrap();
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
    // tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = "I use an account key named {word}")]
async fn create_transaction_signing_key(world: &mut TariWorld, name: String) {
    validator_node_cli::create_or_use_key(world, name);
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

#[when(expr = r#"I submit a transaction manifest on {word} named "{word}" signed with key {word}"#)]
async fn submit_manifest(world: &mut TariWorld, step: &Step, vn_name: String, output_name: String, key_name: String) {
    let manifest = wrap_manifest_in_main(world, step.docstring.as_ref().expect("manifest code not provided"));
    validator_node_cli::submit_manifest(world, vn_name, output_name, manifest, String::new(), key_name).await;
}

#[when(
    regex = r#"^I submit a transaction manifest on (\w+) with inputs "([^"]+)" named "(\w+)" signed with key (\w+)$"#
)]
async fn submit_manifest_with_inputs(
    world: &mut TariWorld,
    step: &Step,
    vn_name: String,
    inputs: String,
    outputs_name: String,
    key_name: String,
) {
    let manifest = wrap_manifest_in_main(world, step.docstring.as_ref().expect("manifest code not provided"));
    validator_node_cli::submit_manifest(world, vn_name, outputs_name, manifest, inputs, key_name).await;
}

#[when(expr = "account {word} reveals {int} burned tokens via wallet daemon {word}")]
async fn reveal_burned_funds(world: &mut TariWorld, account_name: String, amount: u64, wallet_daemon_name: String) {
    wallet_daemon_cli::reveal_burned_funds(world, account_name, amount, wallet_daemon_name).await;
}

#[when(regex = r#"^I submit a transaction manifest via wallet daemon (\w+) with inputs "([^"]+)" named "(\w+)"$"#)]
async fn submit_transaction_manifest_via_wallet_daemon(
    world: &mut TariWorld,
    step: &Step,
    wallet_daemon_name: String,
    inputs: String,
    outputs_name: String,
) {
    let manifest = wrap_manifest_in_main(world, step.docstring.as_ref().expect("manifest code not provided"));
    wallet_daemon_cli::submit_manifest(world, wallet_daemon_name, manifest, inputs, outputs_name, None, None).await;
}

#[when(
    regex = r#"^I submit a transaction manifest via wallet daemon (\w+) signed by the key of (\w+) with inputs "([^"]+)" named "(\w+)"$"#
)]
async fn submit_transaction_manifest_via_wallet_daemon_with_signing_keys(
    world: &mut TariWorld,
    step: &Step,
    wallet_daemon_name: String,
    account_signing_key: String,
    inputs: String,
    outputs_name: String,
) {
    let manifest = wrap_manifest_in_main(world, step.docstring.as_ref().expect("manifest code not provided"));
    wallet_daemon_cli::submit_manifest_with_signing_keys(
        world,
        wallet_daemon_name,
        account_signing_key,
        manifest,
        inputs,
        outputs_name,
        None,
        None,
    )
    .await;
}

#[when(expr = "I mint a new non fungible token {word} on {word} using wallet daemon {word}")]
async fn mint_new_nft_on_account(
    world: &mut TariWorld,
    nft_name: String,
    account_name: String,
    wallet_daemon_name: String,
) {
    wallet_daemon_cli::mint_new_nft_on_account(world, nft_name, account_name, wallet_daemon_name, None, None).await;
}

#[when(expr = "I mint a new non fungible token {word} on {word} using wallet daemon with metadata {word}")]
async fn mint_new_nft_on_account_with_metadata(
    world: &mut TariWorld,
    nft_name: String,
    account_name: String,
    wallet_daemon_name: String,
    metadata: String,
) {
    let metadata = serde_json::from_str::<serde_json::Value>(&metadata).expect("Failed to parse metadata");
    wallet_daemon_cli::mint_new_nft_on_account(world, nft_name, account_name, wallet_daemon_name, None, Some(metadata))
        .await;
}

fn wrap_manifest_in_main(world: &TariWorld, contents: &str) -> String {
    // define all templates
    let template_defs = world.templates.iter().fold(String::new(), |acc, (name, template)| {
        format!("{}\nuse template_{} as {};", acc, template.address, name)
    });
    format!("{} fn main() {{ {} }}", template_defs, contents)
}

#[given(expr = "all validator nodes are connected to each other")]
async fn given_all_validator_connects_to_other_vns(world: &mut TariWorld) {
    let details = world
        .validator_nodes
        .values()
        .map(|vn| {
            (
                vn.public_key.clone(),
                Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", vn.port)).unwrap(),
            )
        })
        .collect::<Vec<_>>();

    for vn in world.validator_nodes.values() {
        if vn.handle.is_finished() {
            eprintln!("Skipping validator node {} that is not running", vn.name);
            continue;
        }
        let mut cli = vn.create_client();
        for (pk, addr) in details.iter().cloned() {
            if pk == vn.public_key {
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
}

#[when(expr = "I wait {int} seconds")]
async fn wait_seconds(_world: &mut TariWorld, seconds: u64) {
    // println!("NOT Waiting {} seconds", seconds);
    tokio::time::sleep(Duration::from_secs(seconds)).await;
}

#[then(expr = "all transactions succeed on all validator nodes")]
async fn successful_transaction(world: &mut TariWorld) {
    // loop over each validator node to check if transaction
    // was accepted by each
    for vn_ps in world.validator_nodes.values() {
        let mut client = vn_ps.create_client();

        let request = GetRecentTransactionsRequest {};
        let recent_transactions_res = client.get_recent_transactions(request).await.unwrap();

        let recent_transactions = recent_transactions_res.transactions;
        // check that all transactions have succeeded
        for tx in &recent_transactions {
            let get_transaction_req = GetTransactionResultRequest {
                transaction_id: *tx.id(),
            };
            let get_transaction_res = client
                .get_transaction_result(get_transaction_req)
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "Failed to get transaction with hash {} for vn = {}",
                        tx.id(),
                        vn_ps.name
                    )
                });
            let finalized_tx = get_transaction_res.result.unwrap_or_else(|| {
                panic!(
                    "Transaction result was rejected for tx hash {} and vn = {}",
                    tx.id(),
                    vn_ps.name
                )
            });
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
    for (name, node) in &world.base_nodes {
        eprintln!(
            "Base node \"{}\": grpc port \"{}\", temp dir path \"{}\"",
            name,
            node.grpc_port,
            node.temp_dir_path.display()
        );
    }

    // wallets
    for (name, node) in &world.wallets {
        eprintln!(
            "Wallet \"{}\": grpc port \"{}\", temp dir path \"{}\"",
            name,
            node.grpc_port,
            node.temp_dir_path.display()
        );
    }

    // vns
    for (name, node) in &world.validator_nodes {
        eprintln!(
            "Validator node \"{}\": json rpc port \"{}\", http ui port \"{}\", temp dir path \"{:?}\"",
            name, node.json_rpc_port, node.http_ui_port, node.temp_dir_path
        );
    }

    // indexes
    for (name, node) in &world.indexers {
        eprintln!(
            "Indexer \"{}\": json rpc port \"{}\", http ui port  \"{}\", temp dir path \"{}\"",
            name, node.json_rpc_port, node.http_ui_port, node.temp_dir_path
        );
    }

    // templates
    for (name, template) in &world.templates {
        eprintln!("Template \"{}\" with address \"{}\"", name, template.address);
    }

    // templates
    for (name, outputs) in &world.outputs {
        eprintln!("Outputs \"{}\"", name);
        for (name, addr) in outputs {
            eprintln!("  - {}: {}", name, addr);
        }
    }

    // wallet daemons
    for (name, daemon) in &world.wallet_daemons {
        eprintln!(
            "Wallet daemon \"{}\": json rpc port \"{}\", indexer jrpc port \"{}\", temp dir path \"{:?}\"",
            name, daemon.json_rpc_port, daemon.indexer_jrpc_port, daemon.temp_path_dir
        );
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
