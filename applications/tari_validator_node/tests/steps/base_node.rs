//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use cucumber::then;

use crate::TariWorld;

#[then(expr = "there is {int} transaction in the mempool of {word} within {int} seconds")]
async fn then_there_is_transaction_in_the_mempool_of(
    world: &mut TariWorld,
    num_tx: usize,
    node: String,
    seconds: usize,
) {
    let node = world.get_base_node(&node);
    let mut client = node.create_client();
    for _ in 0..seconds {
        let mempool_count = client
            .get_mempool_transaction_count()
            .await
            .expect("failed to get mempool count");
        if mempool_count == num_tx {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    let mempool_count = client
        .get_mempool_transaction_count()
        .await
        .expect("failed to get mempool count");
    assert_eq!(mempool_count, num_tx);
}
