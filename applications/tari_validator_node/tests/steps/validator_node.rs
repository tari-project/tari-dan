//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use cucumber::then;
use tari_crypto::tari_utilities::hex::Hex;
use tari_dan_common_types::ShardId;
use tari_engine_types::substate::SubstateAddress;
use tari_validator_node_client::types::GetStateRequest;

use crate::TariWorld;

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
    let mut client = vn.create_client().await;
    let shard_id = ShardId::from_address(
        &SubstateAddress::from_str(state_address).expect("Invalid state address"),
        0,
    );
    client.get_state(GetStateRequest { shard_id }).await.unwrap();
}
