//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use crate::TariWorld;

#[then(expr = "there is {int} transaction(s) in the mempool of {word}")]
async fn then_there_is_transaction_in_the_mempool_of(world: &mut TariWorld, num_tx: usize, node: &str) {
    let node = world.get_node(node).unwrap();
    let mempool = node.create_client().
    assert_eq!(mempool.len(), num_tx);
}
