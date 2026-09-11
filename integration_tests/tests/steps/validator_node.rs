//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{
    str::FromStr,
    time::{Duration, Instant},
};

use cucumber::{gherkin::Step, given, then, when};
use integration_tests::{
    TariWorld,
    base_node::get_base_node_client,
    cucumber_log,
    template,
    template::{RegisteredTemplate, send_template_registration},
    validator_node::{ValidatorNodeProcess, spawn_validator_node},
};
use libp2p::Multiaddr;
use minotari_app_grpc::tari_rpc::{RegisterValidatorNodeRequest, Signature};
use tari_base_node_client::{BaseNodeClient, grpc::GrpcBaseNodeClient};
use tari_crypto::tari_utilities::ByteArray;
use tari_ootle_common_types::{Epoch, SubstateAddress, optional::Optional};
use tari_ootle_storage::Ordering;
use tari_transaction_components::transaction_components::{MemoField, memo_field::TxType};
use tari_validator_node_client::types::{
    AddPeerRequest,
    GetBlocksRequest,
    GetStateRequest,
    GetTemplateRequest,
    ListBlocksRequest,
};
use tonic::codegen::tokio_stream::StreamExt;

async fn spawn_seed_node(
    world: &mut TariWorld,
    seed_vn_name: String,
    bn_name: String,
    claim_fee_account: Option<&str>,
) -> ValidatorNodeProcess {
    let validator = spawn_validator_node(world, seed_vn_name.clone(), bn_name, claim_fee_account).await;
    // Ensure any existing nodes know about the new seed node
    let mut client = validator.get_client();
    let ident = client.get_identity().await.unwrap();
    for vn in world.validator_nodes.values() {
        let mut client = vn.get_client();
        client
            .add_peer(AddPeerRequest {
                public_key: ident.public_key,
                addresses: ident.public_addresses.clone(),
                wait_for_dial: false,
            })
            .await
            .unwrap();
    }
    for indexer in world.indexers.values() {
        indexer.add_peer(ident.public_key, ident.public_addresses.clone()).await;
    }

    validator
}

#[given(expr = "a validator node {word} connected to base node {word}")]
async fn start_vn_without_claim_fee(world: &mut TariWorld, step: &Step, seed_vn_name: String, bn_name: String) {
    cucumber_log!("==== Step: {}", step.value);
    let validator = spawn_validator_node(world, seed_vn_name.clone(), bn_name, None).await;
    world.validator_nodes.insert(seed_vn_name, validator);
}

#[given(expr = "a seed validator node {word} connected to base node {word}")]
async fn start_seed_vn_without_claim_fee(world: &mut TariWorld, step: &Step, seed_vn_name: String, bn_name: String) {
    cucumber_log!("==== Step: {}", step.value);
    let validator = spawn_seed_node(world, seed_vn_name.clone(), bn_name, None).await;
    world.vn_seeds.insert(seed_vn_name, validator);
}

#[given(expr = "a seed validator node {word} connected to base node {word} using claim fee account {word}")]
async fn start_seed_validator_node(
    world: &mut TariWorld,
    step: &Step,
    seed_vn_name: String,
    bn_name: String,
    claim_fee_account: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let validator = spawn_seed_node(world, seed_vn_name.clone(), bn_name, Some(&claim_fee_account)).await;
    world.vn_seeds.insert(seed_vn_name, validator);
}

#[given(expr = "validator {word} nodes connect to all other validators")]
async fn given_validator_connects_to_other_vns(world: &mut TariWorld, step: &Step, name: String) {
    cucumber_log!("==== Step: {}", step.value);
    let details = world
        .all_running_validators_iter()
        .filter(|vn| vn.name != name)
        .map(|vn| {
            (
                vn.public_key,
                Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", vn.p2p_port)).unwrap(),
            )
        })
        .collect::<Vec<_>>();

    let vn = world.validator_nodes.get_mut(&name).unwrap();
    let mut cli = vn.create_client();
    for (pk, addr) in details {
        if let Err(err) = cli
            .add_peer(AddPeerRequest {
                public_key: pk,
                addresses: vec![addr],
                wait_for_dial: true,
            })
            .await
        {
            // TODO: investigate why this can fail. This call failing ("cannot assign requested address (os error 99)")
            // doesnt cause the rest of the test test to fail, so ignoring for now.
            cucumber_log!("Failed to add peer: {}", err);
        }
    }
}

#[when(expr = "validator node {word} sends a registration transaction to base wallet {word}")]
pub async fn send_vn_registration(world: &mut TariWorld, step: &Step, vn_name: String, base_wallet_name: String) {
    cucumber_log!("==== Step: {}", step.value);
    let vn = world.get_validator_node(&vn_name);

    let mut base_layer_wallet = world.get_wallet(&base_wallet_name).create_client().await;
    world.mark_point_in_logs("before get_registration_info");
    let info = vn.get_registration_info().await;
    let registration = info.payload;

    let response = base_layer_wallet
        .register_validator_node(RegisterValidatorNodeRequest {
            validator_node_public_key: registration.public_key.to_vec(),
            validator_node_signature: Some(Signature {
                public_nonce: registration.signature.public_nonce().as_bytes().to_vec(),
                signature: registration.signature.signature().as_bytes().to_vec(),
            }),
            max_epoch: registration.max_epoch.as_u64(),
            validator_node_claim_public_key: registration.claim_public_key.as_bytes().to_vec(),
            sidechain_deployment_key: registration
                .sidechain_public_key
                .map(|key| key.to_vec())
                .unwrap_or_default(),
            fee_per_gram: 1,
            payment_id: MemoField::new_open_from_string("Register by cucumber", TxType::ValidatorNodeRegistration)
                .unwrap()
                .to_bytes(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        response.is_success,
        "Failed to register validator node {}",
        response.failure_message
    );
    integration_tests::cucumber_log!("Validator node registration tx id: {}", response.transaction_id);

    world
        .wait_until_base_nodes_have_transaction_in_mempool(1, Duration::from_secs(10))
        .await;
    world.mark_point_in_logs("after register_validator_node");
}

#[when(expr = "wallet daemon {word} publishes the template \"{word}\" using account {word}")]
async fn publish_template(
    world: &mut TariWorld,
    step: &Step,
    wallet_daemon_name: String,
    template_name: String,
    account_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    world.mark_point_in_logs("Start publishing template");
    let template_address =
        match template::publish_template(world, wallet_daemon_name, account_name, template_name.clone()).await {
            Ok(resp) => resp,
            Err(e) => {
                cucumber_log!("publish_template error = {}", e);
                panic!("publish_template error = {}", e);
            },
        };
    assert!(!template_address.is_empty());

    // store the template address for future reference
    let registered_template = RegisteredTemplate {
        name: template_name.clone(),
        address: template_address,
    };
    world.templates.insert(template_name, registered_template);

    world.mark_point_in_logs("End publishing template");
}

#[when(expr = "base wallet {word} registers the template \"{word}\"")]
async fn register_template(world: &mut TariWorld, step: &Step, wallet_name: String, template_name: String) {
    cucumber_log!("==== Step: {}", step.value);
    world.mark_point_in_logs("Start register template");
    let template_address = match send_template_registration(world, template_name.clone(), wallet_name).await {
        Ok(resp) => resp,
        Err(e) => {
            cucumber_log!("register_template error = {}", e);
            panic!("register_template error = {}", e);
        },
    };
    assert!(!template_address.is_empty());

    // store the template address for future reference
    let registered_template = RegisteredTemplate {
        name: template_name.clone(),
        address: template_address,
    };
    world.templates.insert(template_name, registered_template);

    world
        .wait_until_base_nodes_have_transaction_in_mempool(1, Duration::from_secs(10))
        .await;
    world.mark_point_in_logs("End register template");
}

#[then(expr = "all validator nodes are listed as registered")]
async fn assert_all_vns_are_registered(world: &mut TariWorld, step: &Step) {
    cucumber_log!("==== Step: {}", step.value);
    for vn_ps in world.all_running_validators_iter() {
        // create a base node client
        let base_node_grpc_port = vn_ps.base_node_grpc_port;
        let mut base_node_client: GrpcBaseNodeClient = get_base_node_client(base_node_grpc_port);

        // get the list of registered vns from the base node
        let height = base_node_client.get_tip_info().await.unwrap().height_of_longest_chain;
        let vns = base_node_client
            .get_validator_nodes(height)
            .await
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .await
            .unwrap();
        assert!(!vns.is_empty());

        // retrieve the VN's public key
        let mut client = vn_ps.get_client();
        let identity = client.get_identity().await.unwrap();

        // check that the vn's public key is in the list of registered vns
        assert!(vns.iter().any(|vn| vn.public_key == identity.public_key));
    }
}

#[then(expr = "the validator node {word} is listed as registered")]
pub async fn assert_vn_is_registered(world: &mut TariWorld, step: &Step, vn_name: String) {
    cucumber_log!("==== Step: {}", step.value);
    // create a base node client
    let vn = world.get_validator_node(&vn_name);
    let mut base_node_client: GrpcBaseNodeClient = get_base_node_client(vn.base_node_grpc_port);

    // get the list of registered vns from the base node
    let height = base_node_client.get_tip_info().await.unwrap().height_of_longest_chain;
    let vns = base_node_client
        .get_validator_nodes(height)
        .await
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .await
        .unwrap();
    assert!(!vns.is_empty(), "vns are empty at height {}", height);

    // retrieve the VN's public key
    let mut client = vn.get_client();
    let identity = client.get_identity().await.unwrap();

    // check that the vn's public key is in the list of registered vns
    assert!(vns.iter().any(|vn| vn.public_key == identity.public_key));

    // The VN scanner lags behind the tip by base_layer_confirmations blocks,
    // so the scanned height will never reach the actual tip height.
    let lagged_height = height.saturating_sub(world.consensus_constants.base_layer_confirmations);
    let mut count = 0;
    loop {
        // wait for the validator to pick up the registration
        let stats = client.get_epoch_manager_stats().await.unwrap();
        if stats.current_block_height >= lagged_height || stats.committee_info.is_some() {
            break;
        }
        if count > 40 {
            panic!(
                "Timed out waiting for validator node to pick up registration (current block height: {}, target \
                 lagged height: {})",
                stats.current_block_height, lagged_height
            );
        }
        count += 1;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[then(expr = "the template \"{word}\" is listed as registered by the validator node {word}")]
async fn assert_template_is_registered(world: &mut TariWorld, step: &Step, template_name: String, vn_name: String) {
    cucumber_log!("==== Step: {}", step.value);
    // give it some time for the template tx to be picked up by the VNs
    // tokio::time::sleep(Duration::from_secs(4)).await;

    // retrieve the template address
    let template_address = world.templates.get(&template_name).unwrap().address;

    // try to get the template from the VN
    let timer = Instant::now();
    let vn = world.get_validator_node(&vn_name);
    let mut client = vn.get_client();
    loop {
        let req = GetTemplateRequest { template_address };
        let resp = client.get_template(req).await.ok();

        if resp.is_none() {
            if timer.elapsed() > Duration::from_secs(120) {
                panic!("Timed out waiting for template to be registered by all VNs");
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        // check that the template is indeed in the response
        assert_eq!(resp.unwrap().metadata.address, template_address);
        break;
    }
}

#[then(expr = "the template \"{word}\" is listed as registered by all validator nodes")]
async fn assert_template_is_registered_by_all(world: &mut TariWorld, step: &Step, template_name: String) {
    cucumber_log!("==== Step: {}", step.value);
    // give it some time for the template tx to be picked up by the VNs
    // tokio::time::sleep(Duration::from_secs(4)).await;

    // retrieve the template address
    let template_address = world.templates.get(&template_name).unwrap().address;

    // try to get the template for each VN
    let timer = Instant::now();
    'outer: loop {
        for vn_ps in world.all_running_validators_iter() {
            let mut client = vn_ps.get_client();
            let req = GetTemplateRequest { template_address };
            let resp = client.get_template(req).await.ok();

            if resp.is_none() {
                if timer.elapsed() > Duration::from_secs(120) {
                    panic!("Timed out waiting for template to be registered by all VNs");
                }

                tokio::time::sleep(Duration::from_secs(1)).await;
                continue 'outer;
            }
            let resp = resp.unwrap();
            // check that the template is indeed in the response
            assert_eq!(resp.metadata.address, template_address);
        }
        break;
    }
}

#[then(expr = "validator node {word} has state at {word} within {int} seconds")]
async fn then_validator_node_has_state_at(
    world: &mut TariWorld,
    step: &Step,
    vn_name: String,
    state_address_name: String,
    timeout_secs: u64,
) {
    cucumber_log!("==== Step: {}", step.value);
    let state_address = world
        .substate_ids
        .get(&state_address_name)
        .unwrap_or_else(|| panic!("Address {} not found", state_address_name));
    integration_tests::cucumber_log!("Waiting for state at address {}", state_address);
    let vn = world.get_validator_node(&vn_name);
    let mut client = vn.create_client();
    let substate_address = SubstateAddress::from_substate_id(state_address, 0);
    let mut attempts = 0;
    loop {
        match client
            .get_state(GetStateRequest {
                address: substate_address,
            })
            .await
            .optional()
            .unwrap()
        {
            Some(_) => return,
            None => {
                attempts += 1;
                if attempts == timeout_secs {
                    panic!("State at address {} not found", state_address);
                }
            },
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[then(expr = "I wait for {word} to have at least {int} blocks for the current epoch")]
async fn vn_has_blocks_for_current_epoch(world: &mut TariWorld, step: &Step, vn_name: String, num_blocks: u64) {
    cucumber_log!("==== Step: {}", step.value);
    const TIMEOUT_SECS: u64 = 60;

    let vn = world.get_validator_node(&vn_name);
    let mut client = vn.create_client();
    let mut last_status = None;

    for _ in 0..TIMEOUT_SECS {
        let status = match client.get_consensus_status().await {
            Ok(status) => status,
            Err(err) => {
                integration_tests::cucumber_log!(
                    "Failed to get consensus status for validator node {} while waiting for at least {} blocks: {}",
                    vn_name,
                    num_blocks,
                    err
                );
                panic!("Failed to get consensus status for validator node {vn_name}: {err}");
            },
        };
        last_status = Some(format!(
            "epoch={}, state={}, height={}",
            status.epoch, status.state, status.height
        ));

        if status.state != "Running" {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        if status.height.as_u64() >= num_blocks {
            return;
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let last_status = last_status.unwrap_or_else(|| "no consensus status was observed".to_string());
    let message = format!(
        "Validator node {} did not reach at least {} blocks for the current epoch within {}s. Last status: {}",
        vn_name, num_blocks, TIMEOUT_SECS, last_status
    );
    integration_tests::cucumber_log!("{}", message);
    panic!("{}", message);
}

#[then(expr = "{word} is on epoch {int} within {int} seconds")]
async fn vn_has_scanned_to_epoch(world: &mut TariWorld, step: &Step, vn_name: String, epoch: u64, seconds: usize) {
    cucumber_log!("==== Step: {}", step.value);
    let epoch = Epoch(epoch);
    let vn = world.get_validator_node(&vn_name);
    let mut client = vn.create_client();
    for _ in 0..seconds {
        let stats = client.get_epoch_manager_stats().await.expect("Failed to get stats");
        if stats.current_epoch == epoch {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    let stats = client.get_epoch_manager_stats().await.expect("Failed to get stats");
    assert_eq!(stats.current_epoch, epoch);
}

#[then(expr = "{word} has scanned to at least height {int}")]
async fn vn_has_scanned_to_height(world: &mut TariWorld, step: &Step, vn_name: String, block_height: u64) {
    cucumber_log!("==== Step: {}", step.value);
    let vn = world.get_validator_node(&vn_name);
    let mut client = vn.create_client();
    let mut last_block_height = 0;
    let mut remaining = 10;
    loop {
        let stats = client.get_epoch_manager_stats().await.expect("Failed to get stats");
        if stats.current_block_height >= block_height {
            return;
        }

        if stats.current_block_height != last_block_height {
            last_block_height = stats.current_block_height;
            // Reset the timer each time the scanned height changes
            remaining = 10;
        }

        if remaining == 0 {
            panic!(
                "Validator {} has not scanned to height {}. Current height: {}",
                vn_name, block_height, stats.current_block_height
            );
        }
        remaining -= 1;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[then(expr = "all validators have scanned to height {int}")]
#[when(expr = "all validators have scanned to height {int}")]
async fn all_vns_have_scanned_to_height(world: &mut TariWorld, step: &Step, block_height: u64) {
    cucumber_log!("==== Step: {}", step.value);
    let all_names = world
        .all_running_validators_iter()
        .filter(|vn| !vn.handle.is_finished())
        .map(|vn| vn.name.clone())
        .collect::<Vec<_>>();
    for vn in all_names {
        vn_has_scanned_to_height(world, step, vn, block_height).await;
    }
}

#[when(expr = "I wait for validator {word} has leaf block height of at least {int} at epoch {int}")]
async fn when_i_wait_for_validator_leaf_block_at_least(
    world: &mut TariWorld,
    step: &Step,
    name: String,
    height: u64,
    epoch: u64,
) {
    cucumber_log!("==== Step: {}", step.value);
    let vn = world.get_validator_node(&name);
    let mut client = vn.create_client();

    // Allow enough time for force_beat to trigger (block_time=10s + delta + latency)
    for _ in 0..120 {
        let epoch_stats = client.get_epoch_manager_stats().await.unwrap();
        let resp = client
            .list_blocks_paginated(GetBlocksRequest {
                limit: 1,
                offset: 0,
                ordering_index: Some(2),
                ordering: Some(Ordering::Descending),
                filter_index: Some(1),
                filter: Some(epoch.to_string()),
            })
            .await
            .unwrap();

        let block_height = resp.blocks.first().map(|b| b.height().as_u64()).unwrap_or(0);

        integration_tests::cucumber_log!(
            "Validator {name} leaf block height at epoch {} is {} (current epoch is {})",
            epoch,
            block_height,
            epoch_stats.current_epoch.as_u64()
        );

        if let Some(block) = resp.blocks.first() {
            assert!(block.epoch().as_u64() <= epoch);
            if block.epoch().as_u64() < epoch {
                cucumber_log!("VN {name} is in {}. Waiting for epoch {epoch}", block.epoch())
            }
            if block.epoch().as_u64() == epoch && block.height().as_u64() >= height {
                return;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let consensus_status = client.get_consensus_status().await.unwrap();
    let block_height = client
        .list_blocks_paginated(GetBlocksRequest {
            limit: 1,
            offset: 0,
            ordering_index: Some(2),
            ordering: Some(Ordering::Descending),
            filter_index: Some(1),
            filter: Some(epoch.to_string()),
        })
        .await
        .map(|r| r.blocks.first().map(|b| b.height().as_u64()).unwrap_or(0))
        .unwrap_or(0);
    panic!(
        "Validator {} leaf block height {} is less than {} at epoch {} (consensus: epoch={}, height={}, state={})",
        name, block_height, height, epoch, consensus_status.epoch, consensus_status.height, consensus_status.state,
    );
}

#[when(expr = "Block height on VN {word} is at least {int}")]
async fn when_block_height(world: &mut TariWorld, step: &Step, vn_name: String, height: u64) {
    cucumber_log!("==== Step: {}", step.value);
    let vn = world.get_validator_node(&vn_name);
    let mut client = vn.create_client();
    for _ in 0..20 {
        if client
            .list_blocks(ListBlocksRequest {
                from_id: None,
                limit: 1,
            })
            .await
            .unwrap()
            .blocks[0]
            .height()
            .as_u64() >=
            height
        {
            return;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    panic!("Block height on VN {vn_name} is less than {height}");
}

#[then(expr = "the validator node {word} has started epoch {int}")]
async fn then_validator_node_switches_epoch(world: &mut TariWorld, step: &Step, vn_name: String, epoch: u64) {
    cucumber_log!("==== Step: {}", step.value);
    let vn = world.get_validator_node(&vn_name);
    let mut client = vn.create_client();
    for _ in 0..200 {
        let list_block = client
            .list_blocks_paginated(GetBlocksRequest {
                limit: 10,
                offset: 0,
                ordering_index: None,
                ordering: None,
                filter_index: Some(1),
                filter: Some(epoch.to_string()),
            })
            .await
            .unwrap();
        let blocks = list_block.blocks;
        assert!(
            blocks.iter().all(|b| b.epoch().as_u64() <= epoch),
            "Epoch is greater than expected"
        );
        if blocks.iter().any(|b| b.epoch().as_u64() == epoch) {
            return;
        }

        tokio::time::sleep(Duration::from_secs(8)).await;
    }
    panic!("Validator node {vn_name} did not switch to epoch {epoch}");
}

#[when(expr = "all validator nodes have started epoch {int}")]
pub async fn all_validators_have_started_epoch(world: &mut TariWorld, step: &Step, epoch: u64) {
    cucumber_log!("==== Step: {}", step.value);
    let validators = world.all_running_validators_iter().collect::<Vec<_>>();
    if validators.is_empty() {
        panic!("No running validator nodes found while waiting for epoch {epoch}");
    }

    let timeout_at = Instant::now() + Duration::from_secs(60);
    loop {
        let mut statuses = Vec::with_capacity(validators.len());
        let mut pending = Vec::new();

        for vn in &validators {
            let mut client = vn.create_client();
            match client.get_consensus_status().await {
                Ok(status) => {
                    statuses.push(format!(
                        "{}: epoch {}, state {}, height {}",
                        vn.name, status.epoch, status.state, status.height
                    ));
                    if status.epoch.as_u64() < epoch {
                        pending.push(vn.name.as_str());
                    }
                },
                Err(err) => {
                    statuses.push(format!("{}: status unavailable ({err})", vn.name));
                    pending.push(vn.name.as_str());
                },
            }
        }

        if pending.is_empty() {
            cucumber_log!(
                "All validator nodes have started epoch {} ({})",
                epoch,
                statuses.join("; ")
            );
            return;
        }

        if Instant::now() >= timeout_at {
            panic!(
                "Validator nodes did not all start epoch {} within 60 seconds. Pending: {}. Last statuses: {}",
                epoch,
                pending.join(", "),
                statuses.join("; ")
            );
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[then(expr = "validator {word} is not a member of the current network according to {word}")]
async fn validator_not_member_of_network(world: &mut TariWorld, step: &Step, validator: String, base_node: String) {
    cucumber_log!("==== Step: {}", step.value);
    let bn = world.get_base_node(&base_node);
    let vn = world.get_validator_node(&validator);
    let mut client = bn.create_client();

    let timeout_at = Instant::now() + Duration::from_secs(30);
    loop {
        let tip = client.get_tip_info().await.unwrap();
        let mut vns = client.get_validator_nodes(tip.height_of_longest_chain).await.unwrap();
        let has_vn = vns.any(|v| v.unwrap().public_key == vn.public_key).await;
        if !has_vn {
            return;
        }
        if Instant::now() >= timeout_at {
            panic!(
                "Validator {} is still a member of the network (height {}) but expected it not to be",
                validator, tip.height_of_longest_chain
            );
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
