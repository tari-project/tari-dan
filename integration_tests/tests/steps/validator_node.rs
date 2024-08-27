//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{
    str::FromStr,
    time::{Duration, Instant},
};

use cucumber::{given, then, when};
use integration_tests::{
    base_node::get_base_node_client,
    template::{send_template_registration, RegisteredTemplate},
    validator_node::spawn_validator_node,
    validator_node_cli::create_key,
    TariWorld,
};
use libp2p::Multiaddr;
use minotari_app_grpc::tari_rpc::{RegisterValidatorNodeRequest, Signature};
use tari_base_node_client::{grpc::GrpcBaseNodeClient, BaseNodeClient};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::{Epoch, SubstateAddress};
use tari_engine_types::substate::SubstateId;
use tari_validator_node_client::types::{AddPeerRequest, GetBlocksRequest, GetStateRequest, GetTemplateRequest};

#[given(expr = "a validator node {word} connected to base node {word} and wallet daemon {word}")]
async fn start_validator_node(world: &mut TariWorld, vn_name: String, bn_name: String, wallet_daemon_name: String) {
    let vn = spawn_validator_node(
        world,
        vn_name.clone(),
        bn_name,
        wallet_daemon_name,
        format!("{}_claim_fee", vn_name),
    )
    .await;
    world.validator_nodes.insert(vn_name, vn);
}

#[given(expr = "a seed validator node {word} connected to base node {word} and wallet daemon {word}")]
async fn start_seed_vn_without_claim_fee(
    world: &mut TariWorld,
    seed_vn_name: String,
    bn_name: String,
    wallet_daemon_name: String,
) {
    start_seed_validator_node(
        world,
        seed_vn_name.clone(),
        bn_name,
        wallet_daemon_name,
        format!("{}_claim_fee", &seed_vn_name),
    )
    .await;
}

#[given(
    expr = "a seed validator node {word} connected to base node {word} and wallet daemon {word} using claim fee key \
            {word}"
)]
async fn start_seed_validator_node(
    world: &mut TariWorld,
    seed_vn_name: String,
    bn_name: String,
    wallet_daemon_name: String,
    claim_fee_key_name: String,
) {
    let validator = spawn_validator_node(
        world,
        seed_vn_name.clone(),
        bn_name,
        wallet_daemon_name,
        claim_fee_key_name,
    )
    .await;
    // Ensure any existing nodes know about the new seed node
    let mut client = validator.get_client();
    let ident = client.get_identity().await.unwrap();
    for vn in world.validator_nodes.values() {
        let mut client = vn.get_client();
        client
            .add_peer(AddPeerRequest {
                public_key: ident.public_key.clone(),
                addresses: ident.public_addresses.clone(),
                wait_for_dial: false,
            })
            .await
            .unwrap();
    }
    for indexer in world.indexers.values() {
        let mut client = indexer.get_jrpc_indexer_client();
        client
            .add_peer(tari_indexer_client::types::AddPeerRequest {
                public_key: ident.public_key.clone(),
                addresses: ident.public_addresses.clone(),
                wait_for_dial: false,
            })
            .await
            .unwrap();
    }

    world.vn_seeds.insert(seed_vn_name, validator);
}

#[given(expr = "{int} validator nodes connected to base node {word} and wallet daemon {word}")]
async fn start_multiple_validator_nodes(world: &mut TariWorld, num_nodes: u64, bn_name: String, wallet_name: String) {
    for i in 1..=num_nodes {
        let vn_name = format!("VAL_{i}");
        let vn = spawn_validator_node(
            world,
            vn_name.clone(),
            bn_name.clone(),
            wallet_name.clone(),
            format!("{}_claim_fee", vn_name),
        )
        .await;
        world.validator_nodes.insert(vn_name, vn);
    }
}

#[given(expr = "validator {word} nodes connect to all other validators")]
async fn given_validator_connects_to_other_vns(world: &mut TariWorld, name: String) {
    let details = world
        .all_validators_iter()
        .filter(|vn| vn.name != name)
        .map(|vn| {
            (
                vn.public_key.clone(),
                Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", vn.port)).unwrap(),
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
            eprintln!("Failed to add peer: {}", err);
        }
    }
}

#[when(expr = "validator node {word} sends a registration transaction to base wallet {word}")]
pub async fn send_vn_registration_with_claim_wallet(world: &mut TariWorld, vn_name: String, base_wallet_name: String) {
    let vn = world.get_validator_node(&vn_name);

    let mut base_layer_wallet = world.get_wallet(&base_wallet_name).create_client().await;
    world.mark_point_in_logs("before get_registration_info");
    let registration = vn.get_registration_info();

    let response = base_layer_wallet
        .register_validator_node(RegisterValidatorNodeRequest {
            validator_node_public_key: registration.public_key.to_vec(),
            validator_node_signature: Some(Signature {
                public_nonce: registration.signature.signature().get_public_nonce().to_vec(),
                signature: registration.signature.signature().get_signature().to_vec(),
            }),
            validator_node_claim_public_key: registration.claim_fees_public_key.to_vec(),
            sidechain_deployment_key: vec![],
            fee_per_gram: 1,
            message: "Register".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        response.is_success,
        "Failed to register validator node {}",
        response.failure_message
    );
    world.mark_point_in_logs("after register_validator_node");
}

#[when(expr = "base wallet {word} registers the template \"{word}\"")]
async fn register_template(world: &mut TariWorld, wallet_name: String, template_name: String) {
    world.mark_point_in_logs("Start register template");
    let template_address = match send_template_registration(world, template_name.clone(), wallet_name).await {
        Ok(resp) => resp,
        Err(e) => {
            println!("register_template error = {}", e);
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
async fn assert_all_vns_are_registered(world: &mut TariWorld) {
    for vn_ps in world.all_validators_iter() {
        // create a base node client
        let base_node_grpc_port = vn_ps.base_node_grpc_port;
        let mut base_node_client: GrpcBaseNodeClient = get_base_node_client(base_node_grpc_port);

        // get the list of registered vns from the base node
        let height = base_node_client.get_tip_info().await.unwrap().height_of_longest_chain;
        let vns = base_node_client.get_validator_nodes(height).await.unwrap();
        assert!(!vns.is_empty());

        // retrieve the VN's public key
        let mut client = vn_ps.get_client();
        let identity = client.get_identity().await.unwrap();

        // check that the vn's public key is in the list of registered vns
        assert!(vns.iter().any(|vn| vn.public_key == identity.public_key));
    }
}

#[then(expr = "the validator node {word} is listed as registered")]
pub async fn assert_vn_is_registered(world: &mut TariWorld, vn_name: String) {
    // create a base node client
    let vn = world.get_validator_node(&vn_name);
    let mut base_node_client: GrpcBaseNodeClient = get_base_node_client(vn.base_node_grpc_port);

    // get the list of registered vns from the base node
    let height = base_node_client.get_tip_info().await.unwrap().height_of_longest_chain;
    let vns = base_node_client.get_validator_nodes(height).await.unwrap();
    assert!(!vns.is_empty());

    // retrieve the VN's public key
    let mut client = vn.get_client();
    let identity = client.get_identity().await.unwrap();

    // check that the vn's public key is in the list of registered vns
    assert!(vns.iter().any(|vn| vn.public_key == identity.public_key));

    let mut count = 0;
    loop {
        // wait for the validator to pick up the registration
        let stats = client.get_epoch_manager_stats().await.unwrap();
        if stats.current_block_height >= height || stats.is_valid {
            break;
        }
        if count > 10 {
            panic!("Timed out waiting for validator node to pick up registration");
        }
        count += 1;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[then(expr = "the template \"{word}\" is listed as registered by the validator node {word}")]
async fn assert_template_is_registered(world: &mut TariWorld, template_name: String, vn_name: String) {
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
            if timer.elapsed() > Duration::from_secs(10) {
                panic!("Timed out waiting for template to be registered by all VNs");
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        // check that the template is indeed in the response
        assert_eq!(resp.unwrap().registration_metadata.address, template_address);
        break;
    }
}

#[then(expr = "the template \"{word}\" is listed as registered by all validator nodes")]
async fn assert_template_is_registered_by_all(world: &mut TariWorld, template_name: String) {
    // give it some time for the template tx to be picked up by the VNs
    // tokio::time::sleep(Duration::from_secs(4)).await;

    // retrieve the template address
    let template_address = world.templates.get(&template_name).unwrap().address;

    // try to get the template for each VN
    let timer = Instant::now();
    'outer: loop {
        for vn_ps in world.all_validators_iter() {
            let mut client = vn_ps.get_client();
            let req = GetTemplateRequest { template_address };
            let resp = client.get_template(req).await.ok();

            if resp.is_none() {
                if timer.elapsed() > Duration::from_secs(10) {
                    panic!("Timed out waiting for template to be registered by all VNs");
                }

                tokio::time::sleep(Duration::from_secs(1)).await;
                continue 'outer;
            }
            let resp = resp.unwrap();
            // check that the template is indeed in the response
            assert_eq!(resp.registration_metadata.address, template_address);
        }
        break;
    }
}

#[then(expr = "validator node {word} has state at {word}")]
async fn then_validator_node_has_state_at(world: &mut TariWorld, vn_name: String, state_address_name: String) {
    let state_address = world
        .addresses
        .get(&state_address_name)
        .unwrap_or_else(|| panic!("Address {} not found", state_address_name));
    let vn = world.get_validator_node(&vn_name);
    let mut client = vn.create_client();
    let substate_address =
        SubstateAddress::from_substate_id(&SubstateId::from_str(state_address).expect("Invalid state address"), 0);
    if let Err(e) = client
        .get_state(GetStateRequest {
            address: substate_address,
        })
        .await
    {
        println!("Failed to get state: {}", e);
        panic!("Failed to get state: {}", e);
    }
}

#[then(expr = "{word} is on epoch {int} within {int} seconds")]
async fn vn_has_scanned_to_epoch(world: &mut TariWorld, vn_name: String, epoch: u64, seconds: usize) {
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

#[then(expr = "{word} has scanned to height {int}")]
async fn vn_has_scanned_to_height(world: &mut TariWorld, vn_name: String, block_height: u64) {
    let vn = world.get_validator_node(&vn_name);
    let mut client = vn.create_client();
    let mut last_block_height = 0;
    let mut remaining = 10;
    loop {
        let stats = client.get_epoch_manager_stats().await.expect("Failed to get stats");
        if stats.current_block_height == block_height {
            return;
        }
        assert!(
            stats.current_block_height <= block_height,
            "Validator {} has scanned past the target height {}. Current height: {}",
            vn_name,
            block_height,
            stats.current_block_height
        );

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
async fn all_vns_have_scanned_to_height(world: &mut TariWorld, block_height: u64) {
    let all_names = world
        .all_validators_iter()
        .map(|vn| vn.name.clone())
        .collect::<Vec<_>>();
    for vn in all_names {
        vn_has_scanned_to_height(world, vn, block_height).await;
    }
}

#[when(expr = "I create a new key pair {word}")]
async fn when_i_create_new_key_pair(world: &mut TariWorld, key_name: String) {
    create_key(world, key_name);
}

#[when(expr = "I wait for validator {word} has leaf block height of at least {int} at epoch {int}")]
async fn when_i_wait_for_validator_leaf_block_at_least(world: &mut TariWorld, name: String, height: u64, epoch: u64) {
    let vn = world.get_validator_node(&name);
    let mut client = vn.create_client();
    for _ in 0..40 {
        let resp = client
            .list_blocks_paginated(GetBlocksRequest {
                limit: 1,
                offset: 0,
                ordering_index: None,
                ordering: None,
                filter_index: None,
                filter: None,
            })
            .await
            .unwrap();

        // for b in resp.blocks.iter() {
        //     eprintln!("----------> {b}");
        // }
        // eprintln!("-----------");

        if let Some(block) = resp.blocks.first() {
            assert!(block.epoch().as_u64() <= epoch);
            if block.epoch().as_u64() == epoch && block.height().as_u64() >= height {
                return;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let resp = client
        .list_blocks_paginated(GetBlocksRequest {
            limit: 1,
            offset: 0,
            ordering_index: None,
            ordering: None,
            filter_index: None,
            filter: None,
        })
        .await
        .unwrap();
    let actual_height = resp.blocks.first().unwrap().height().as_u64();
    if actual_height < height {
        panic!(
            "Validator {} leaf block height {} is less than {}",
            name, actual_height, height
        );
    }
}

#[when(expr = "Block count on VN {word} is at least {int}")]
async fn when_count(world: &mut TariWorld, vn_name: String, count: u64) {
    let vn = world.get_validator_node(&vn_name);
    let mut client = vn.create_client();
    for _ in 0..20 {
        if client.get_blocks_count().await.unwrap().count as u64 >= count {
            return;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    panic!("Block count on VN {vn_name} is less than {count}");
}

#[then(expr = "the validator node {word} switches to epoch {int}")]
async fn then_validator_node_switches_epoch(world: &mut TariWorld, vn_name: String, epoch: u64) {
    let vn = world.get_validator_node(&vn_name);
    let mut client = vn.create_client();
    for _ in 0..200 {
        let list_block = client
            .list_blocks_paginated(GetBlocksRequest {
                limit: 10,
                offset: 0,
                ordering_index: None,
                ordering: None,
                filter_index: None,
                filter: None,
            })
            .await
            .unwrap();
        let blocks = list_block.blocks;
        assert!(
            blocks.iter().all(|b| b.epoch().as_u64() <= epoch),
            "Epoch is greater than expected"
        );
        if blocks.iter().any(|b| b.epoch().as_u64() == epoch) {
            assert!(blocks.iter().any(|b| b.is_epoch_end()), "No end epoch block found");
            return;
        }

        tokio::time::sleep(Duration::from_secs(8)).await;
    }
    panic!("Validator node {vn_name} did not switch to epoch {epoch}");
}
