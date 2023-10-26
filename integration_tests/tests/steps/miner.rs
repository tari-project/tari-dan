//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use cucumber::{given, when};
use tari_base_node_client::BaseNodeClient;

use crate::{mine_blocks, register_miner_process, TariWorld};

#[given(expr = "a miner {word} connected to base node {word} and wallet {word}")]
async fn create_miner(world: &mut TariWorld, miner_name: String, bn_name: String, wallet_name: String) {
    register_miner_process(world, miner_name, bn_name, wallet_name);
}

#[when(expr = "miner {word} mines {int} new blocks")]
async fn miner_mines_new_blocks(world: &mut TariWorld, miner_name: String, num_blocks: u64) {
    let bn = world
        .base_nodes
        .values()
        .next()
        .expect("Cannot mine because there are no base nodes");
    let mut client = bn.create_client();
    let start_tip = client.get_tip_info().await.unwrap().height_of_longest_chain;

    mine_blocks(world, miner_name, num_blocks).await;

    // wait for all tips to be the new height
    for bn in world.base_nodes.values() {
        let mut client = bn.create_client();
        let mut tip = client.get_tip_info().await.unwrap();
        let mut iter_count = 0;
        while tip.height_of_longest_chain < start_tip + num_blocks {
            tip = client.get_tip_info().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
            if iter_count > 100 {
                panic!("Timed out waiting for tip height to reach {}", start_tip + num_blocks);
            }
            iter_count += 1;
        }
    }
}
