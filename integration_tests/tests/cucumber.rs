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
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    future,
    io,
    panic,
    str::FromStr,
    time::Duration,
};

use cucumber::{gherkin::Step, given, then, when, writer, writer::Verbosity, World, WriterExt};
use integration_tests::{
    base_node::spawn_base_node,
    http_server::{spawn_template_http_server, MockHttpServer},
    indexer::{spawn_indexer, IndexerProcess},
    logging::{create_log_config_file, get_base_dir},
    miner::{mine_blocks, register_miner_process},
    validator_node::spawn_validator_node,
    validator_node_cli,
    wallet::spawn_wallet,
    wallet_daemon::spawn_wallet_daemon,
    wallet_daemon_cli,
    TariWorld,
};
use tari_common::initialize_logging;
use tari_comms::multiaddr::Multiaddr;
use tari_crypto::tari_utilities::hex::Hex;
use tari_dan_engine::abi::Type;
use tari_dan_storage::consensus_models::QuorumDecision;
use tari_shutdown::Shutdown;
use tari_validator_node_client::types::{AddPeerRequest, GetRecentTransactionsRequest, GetTransactionResultRequest};

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
            world.current_scenario_name = Some(scenario.name.clone());
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

#[given(expr = "a base node {word}")]
async fn start_base_node(world: &mut TariWorld, bn_name: String) {
    spawn_base_node(world, bn_name).await;
}

#[given(expr = "fees are enabled")]
async fn fees_are_enabled(world: &mut TariWorld) {
    world.fees_enabled = true;
}

#[given(expr = "a validator node {word} connected to base node {word} and wallet {word}")]
async fn start_validator_node(world: &mut TariWorld, vn_name: String, bn_name: String, wallet_name: String) {
    let vn = spawn_validator_node(world, vn_name.clone(), bn_name, wallet_name).await;
    world.validator_nodes.insert(vn_name, vn);
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
    // tokio::time::sleep(Duration::from_secs(4)).await;
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
    _num_outputs: u64,
    outputs_name: String,
) {
    let args = args.split(',').map(|a| a.trim().to_string()).collect();
    validator_node_cli::create_component(world, outputs_name, template_name, vn_name, function_call, args, vec![])
        .await;

    // give it some time between transactions
    // tokio::time::sleep(Duration::from_secs(4)).await;
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
    _num_outputs: u64,
    outputs_name: String,
    new_resource_token: String,
) {
    let args = args.split(',').map(|a| a.trim().to_string()).collect();
    validator_node_cli::create_component(world, outputs_name, template_name, vn_name, function_call, args, vec![
        new_resource_token,
    ])
    .await;

    // give it some time between transactions
    // tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(
    expr = r#"I call function "{word}" on template "{word}" on {word} with {int} outputs named "{word}" with new resource "{word}""#
)]
async fn call_template_constructor_with_no_args(
    world: &mut TariWorld,
    function_call: String,
    template_name: String,
    vn_name: String,
    _num_outputs: u64,
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
        vec![new_resource_token_symbol],
    )
    .await;

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
    validator_node_cli::create_component(
        world,
        component_name,
        template_name,
        vn_name,
        function_call,
        vec![],
        vec![],
    )
    .await;

    // give it some time between transactions
    // tokio::time::sleep(Duration::from_secs(4)).await;
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
        vec![new_resource_token_symbol],
    )
    .await;

    // give it some time between transactions
    // tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(expr = r#"I invoke on {word} on component {word} the method call "{word}" with {int} outputs named "{word}""#)]
async fn call_component_method(
    world: &mut TariWorld,
    vn_name: String,
    component_name: String,
    method_call: String,
    _num_outputs: u64,
    output_name: String,
) {
    let resp = validator_node_cli::call_method(world, vn_name, component_name, output_name, method_call).await;
    assert_eq!(resp.dry_run_result.unwrap().decision, QuorumDecision::Accept);

    // give it some time between transactions
    // tokio::time::sleep(Duration::from_secs(4)).await;
}

#[when(
    expr = r#"I invoke on all validator nodes on component {word} the method call "{word}" with {int} outputs named "{word}""#
)]
async fn call_component_method_on_all_vns(
    world: &mut TariWorld,
    component_name: String,
    method_call: String,
    _num_outputs: u64,
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
        .await;
        assert_eq!(resp.dry_run_result.unwrap().decision, QuorumDecision::Accept);
    }
    // give it some time between transactions
    // tokio::time::sleep(Duration::from_secs(4)).await;
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
    _num_outputs: u64,
    expected_result: String,
) {
    let resp =
        validator_node_cli::call_method(world, vn_name, component_name, "dummy_outputs".to_string(), method_call).await;
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
    expr = "I invoke on all validator nodes on component {word} the method call \"{word}\" with {int} outputs the \
            result is \"{word}\""
)]
async fn call_component_method_on_all_vns_and_check_result(
    world: &mut TariWorld,
    component_name: String,
    method_call: String,
    _num_outputs: u64,
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
        .await;
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

#[when(expr = r#"I submit a transaction manifest on {word} with {int} outputs named "{word}" signed with key {word}"#)]
async fn submit_manifest(
    world: &mut TariWorld,
    step: &Step,
    vn_name: String,
    // TODO: remove
    _num_outputs: u64,
    output_name: String,
    key_name: String,
) {
    let manifest = wrap_manifest_in_main(world, step.docstring.as_ref().expect("manifest code not provided"));
    validator_node_cli::submit_manifest(world, vn_name, output_name, manifest, String::new(), key_name).await;
}

#[when(
    regex = r#"^I submit a transaction manifest on (\w+) with inputs "([^"]+)" and (\d+) outputs? named "(\w+)" signed with key (\w+)$"#
)]
async fn submit_manifest_with_inputs(
    world: &mut TariWorld,
    step: &Step,
    vn_name: String,
    inputs: String,
    // TODO: remove
    _num_outputs: u64,
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

#[when(expr = "I mint a new non fungible token {word} on {word} using wallet daemon {word}")]
async fn mint_new_nft_on_account(
    world: &mut TariWorld,
    nft_name: String,
    account_name: String,
    wallet_daemon_name: String,
) {
    wallet_daemon_cli::mint_new_nft_on_account(world, nft_name, account_name, wallet_daemon_name, None).await;
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
    wallet_daemon_cli::mint_new_nft_on_account(world, nft_name, account_name, wallet_daemon_name, Some(metadata)).await;
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
    let component_address = [0u8; 32];
    let template_address = [0u8; 32];
    let tx_hash = [0u8; 32];
    let query = format!(
        "{{ getEventsForTransaction(txHash: {:?}) {{ componentAddress, templateAddress, txHash, topic, payload }}
    }}",
        tx_hash.to_hex()
    );
    let res = graphql_client
        .send_request::<HashMap<String, Vec<tari_indexer::graphql::model::events::Event>>>(&query, None, None)
        .await
        .expect("Failed to obtain getEventsForTransaction query result");
    let res = res.get("getEventsForTransaction").unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].component_address, Some(component_address));
    assert_eq!(res[0].template_address, template_address);
    assert_eq!(res[0].tx_hash, tx_hash);
    assert_eq!(res[0].topic, "my_event");
    assert_eq!(
        res[0].payload,
        BTreeMap::from([("my".to_string(), "event".to_string())])
    );
}

#[when(expr = "indexer {word} scans the network {int} events for account {word} with topics {word}")]
async fn indexer_scans_network_events(
    world: &mut TariWorld,
    indexer_name: String,
    num_events: u32,
    account_name: String,
    topics: String,
) {
    let indexer: &mut IndexerProcess = world.indexers.get_mut(&indexer_name).unwrap();
    let accounts_component_addresses = world.outputs.get(&account_name).expect("Account name not found");
    let component_address = accounts_component_addresses
        .into_iter()
        .find(|(k, _)| k.contains("components/Account"))
        .map(|(_, v)| {
            v.address
                .as_component_address()
                .expect("Failed to parse `ComponentAddress`")
        })
        .expect("Did not find component address");

    let mut graphql_client = indexer.get_graphql_indexer_client().await;
    let query = format!(
        "{{ getEventsForComponent(componentAddress: {:?}, version: 0) {{ componentAddress, templateAddress, txHash, \
         topic, payload }} }}",
        component_address.to_string()
    );
    let res = graphql_client
        .send_request::<HashMap<String, Vec<tari_indexer::graphql::model::events::Event>>>(&query, None, None)
        .await
        .expect("Failed to obtain getEventsForComponent query result");

    let events_for_component = res.get("getEventsForComponent").unwrap();
    assert_eq!(events_for_component.len(), num_events as usize);

    let topics = topics.split(',').collect::<Vec<_>>();
    assert_eq!(topics.len(), num_events as usize);

    for (ind, topic) in topics.iter().enumerate() {
        let event = events_for_component[ind].clone();
        assert_eq!(&event.topic, topic);
    }
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
        eprintln!("Wallet daemons \"{}\"", name);
        eprintln!("  - {}: {}", name, daemon.name);
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
