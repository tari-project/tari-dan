//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use cucumber::given;
use integration_tests::{
    base_node::spawn_base_node,
    miner::register_miner_process,
    validator_node::spawn_validator_node,
    wallet::spawn_wallet,
};

use crate::{
    steps::{miner, validator_node, wallet},
    TariWorld,
};

#[given(expr = "a network with registered validator {word} and wallet daemon {word}")]
async fn start_a_network(world: &mut TariWorld, vn_name: String, walletd_name: String) {
    const BASE_NODE_NAME: &str = "NETWORK_BASE";
    const WALLET_NAME: &str = "NETWORK_CONSOLE_WALLET";
    const MINER_NAME: &str = "NETWORK_MINER";

    spawn_base_node(world, BASE_NODE_NAME.to_string()).await;
    spawn_wallet(world, WALLET_NAME.to_string(), BASE_NODE_NAME.to_string()).await;
    register_miner_process(
        world,
        MINER_NAME.to_string(),
        BASE_NODE_NAME.to_string(),
        WALLET_NAME.to_string(),
    );
    let vn = spawn_validator_node(
        world,
        vn_name.clone(),
        BASE_NODE_NAME.to_string(),
        walletd_name,
        format!("{}_claim_fee", vn_name),
    )
    .await;
    world.validator_nodes.insert(vn_name.clone(), vn);

    miner::miner_mines_new_blocks(world, MINER_NAME.to_string(), 6).await;
    wallet::check_balance(world, WALLET_NAME.to_string(), 20, "T".to_string()).await;
    validator_node::send_vn_registration_with_claim_wallet(world, vn_name.clone(), WALLET_NAME.to_string()).await;
    miner::miner_mines_new_blocks(world, MINER_NAME.to_string(), 16).await;
    validator_node::assert_vn_is_registered(world, vn_name).await;
}
