//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use cucumber::{then, when};
use integration_tests::{validator_node_cli::create_key, TariWorld};
use tari_dan_common_types::{Epoch, ShardId};
use tari_engine_types::substate::SubstateAddress;
use tari_validator_node_client::types::GetStateRequest;

#[then(expr = "validator node {word} has state at {word}")]
async fn then_validator_node_has_state_at(world: &mut TariWorld, vn_name: String, state_address_name: String) {
    let state_address = world
        .addresses
        .get(&state_address_name)
        .unwrap_or_else(|| panic!("Address {} not found", state_address_name));
    let vn = world
        .validator_nodes
        .get(&vn_name)
        .unwrap_or_else(|| panic!("Validator node {} not found", vn_name));
    let mut client = vn.create_client();
    let shard_id = ShardId::from_address(
        &SubstateAddress::from_str(state_address).expect("Invalid state address"),
        0,
    );
    if let Err(e) = client.get_state(GetStateRequest { shard_id }).await {
        println!("Failed to get state: {}", e);
        panic!("Failed to get state: {}", e);
    }
}

#[then(expr = "{word} is on epoch {int} within {int} seconds")]
async fn vn_has_scanned_to_epoch(world: &mut TariWorld, vn_name: String, epoch: u64, seconds: usize) {
    let epoch = Epoch(epoch);
    let vn = world
        .validator_nodes
        .get(&vn_name)
        .unwrap_or_else(|| panic!("Validator node {} not found", vn_name));
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

#[then(expr = "{word} has scanned to height {int} within {int} seconds")]
async fn vn_has_scanned_to_height(world: &mut TariWorld, vn_name: String, block_height: u64, seconds: usize) {
    let vn = world
        .validator_nodes
        .get(&vn_name)
        .unwrap_or_else(|| panic!("Validator node {} not found", vn_name));
    let mut client = vn.create_client();
    for _ in 0..seconds {
        let stats = client.get_epoch_manager_stats().await.expect("Failed to get stats");
        if stats.current_block_height == block_height {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    let stats = client.get_epoch_manager_stats().await.expect("Failed to get stats");
    assert_eq!(stats.current_block_height, block_height);
}

#[when(expr = "I create a new key pair {word}")]
async fn when_i_create_new_key_pair(world: &mut TariWorld, key_name: String) {
    create_key(world, key_name);
}
