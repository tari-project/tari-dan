//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use cucumber::{then, when};
use integration_tests::TariWorld;
use tari_comms::multiaddr::Multiaddr;
use tari_indexer_client::types::AddPeerRequest;

#[when(expr = "indexer {word} connects to all other validators")]
async fn given_validator_connects_to_other_vns(world: &mut TariWorld, name: String) {
    let indexer = world.get_indexer(&name);
    let details = world.all_validators_iter().filter(|vn| vn.name != name).map(|vn| {
        (
            vn.public_key.clone(),
            Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", vn.port)).unwrap(),
        )
    });

    let mut cli = indexer.get_jrpc_indexer_client();
    for (pk, addr) in details {
        cli.add_peer(AddPeerRequest {
            public_key: pk,
            addresses: vec![addr],
            wait_for_dial: true,
        })
        .await
        .unwrap();
    }
}

#[then(expr = "indexer {word} has scanned to height {int} within {int} seconds")]
async fn indexer_has_scanned_to_height(world: &mut TariWorld, name: String, block_height: u64, seconds: usize) {
    let indexer = world.get_indexer(&name);
    let mut client = indexer.get_jrpc_indexer_client();
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
