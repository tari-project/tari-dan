//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use cucumber::{given, when};

use crate::{mine_blocks, register_miner_process, TariWorld};

#[given(expr = "a miner {word} connected to base node {word} and wallet {word}")]
async fn create_miner(world: &mut TariWorld, miner_name: String, bn_name: String, wallet_name: String) {
    register_miner_process(world, miner_name, bn_name, wallet_name);
}

#[when(expr = "miner {word} mines {int} new blocks")]
async fn miner_mines_new_blocks(world: &mut TariWorld, miner_name: String, num_blocks: u64) {
    mine_blocks(world, miner_name, num_blocks).await;
}
