//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use cucumber::when;
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
