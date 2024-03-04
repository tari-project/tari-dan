//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeMap, HashMap},
    str::FromStr,
};

use cucumber::{given, then, when};
use integration_tests::{
    indexer::{spawn_indexer, IndexerProcess},
    TariWorld,
};
use libp2p::Multiaddr;
use tari_crypto::tari_utilities::hex::Hex;
use tari_indexer_client::types::AddPeerRequest;
use tari_template_lib::models::ObjectKey;

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
            log::error!("Failed to add peer: {}", err);
        }
    }
}

#[then(expr = "indexer {word} has scanned to height {int}")]
async fn indexer_has_scanned_to_height(world: &mut TariWorld, name: String, block_height: u64) {
    let indexer = world.get_indexer(&name);
    let mut client = indexer.get_jrpc_indexer_client();
    let mut last_block_height = 0;
    let mut remaining = 10;
    loop {
        let stats = client.get_epoch_manager_stats().await.expect("Failed to get stats");
        if stats.current_block_height == block_height {
            return;
        }

        assert!(
            stats.current_block_height <= block_height,
            "Indexer {} has scanned past block height {} to height {}",
            name,
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
                "Indexer {} did not scan to block height {}. Current height: {}",
                name, block_height, stats.current_block_height
            );
        }
        remaining -= 1;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[given(expr = "an indexer {word} connected to base node {word}")]
async fn start_indexer(world: &mut TariWorld, indexer_name: String, bn_name: String) {
    spawn_indexer(world, indexer_name, bn_name).await;
}

#[given(expr = "{word} indexer GraphQL request works")]
async fn works_indexer_graphql(world: &mut TariWorld, indexer_name: String) {
    let indexer: &mut IndexerProcess = world.indexers.get_mut(&indexer_name).unwrap();
    // insert event mock data in the substate manager database
    indexer.insert_event_mock_data().await;
    let mut graphql_client = indexer.get_graphql_indexer_client().await;
    let template_address = [0u8; 32];
    let tx_hash = [0u8; 32];
    let query = format!(
        "{{ getEventsForTransaction(txHash: {:?}) {{ componentAddress, templateAddress, txHash, topic, payload }}
    }}",
        tx_hash.to_hex()
    );
    let res = graphql_client
        .send_request::<HashMap<String, Vec<tari_indexer::graphql::model::events::Event>>>(&query, None, None)
        .await
        .expect("Failed to obtain getEventsForTransaction query result");
    let res = res.get("getEventsForTransaction").unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].component_address, Some(ObjectKey::default().into_array()));
    assert_eq!(res[0].template_address, template_address);
    assert_eq!(res[0].tx_hash, tx_hash);
    assert_eq!(res[0].topic, "my_event");
    assert_eq!(
        res[0].payload,
        BTreeMap::from([("my".to_string(), "event".to_string())])
    );
}

#[when(expr = "indexer {word} scans the network {int} events for account {word} with topics {word}")]
async fn indexer_scans_network_events(
    world: &mut TariWorld,
    indexer_name: String,
    num_events: u32,
    account_name: String,
    topics: String,
) {
    let indexer: &mut IndexerProcess = world.indexers.get_mut(&indexer_name).unwrap();
    let accounts_component_addresses = world.outputs.get(&account_name).expect("Account name not found");
    let component_address = accounts_component_addresses
        .into_iter()
        .find(|(k, _)| k.contains("components/Account"))
        .map(|(_, v)| v)
        .expect("Did not find component address");

    let mut graphql_client = indexer.get_graphql_indexer_client().await;
    let query = format!(
        r#"{{ getEventsForComponent(componentAddress: "{}", version: {}) {{ componentAddress, templateAddress, txHash, topic, payload }} }}"#,
        component_address.substate_id,
        component_address.version.unwrap()
    );
    let res = graphql_client
        .send_request::<HashMap<String, Vec<tari_indexer::graphql::model::events::Event>>>(&query, None, None)
        .await
        .expect("Failed to obtain getEventsForComponent query result");

    let events_for_component = res.get("getEventsForComponent").unwrap();
    assert_eq!(
        events_for_component.len(),
        num_events as usize,
        "Unexpected number of events returned got {}, expected {}",
        events_for_component.len(),
        num_events
    );

    let topics = topics.split(',').collect::<Vec<_>>();
    assert_eq!(
        topics.len(),
        num_events as usize,
        "Unexpected number of topics provided got {}, expected {}",
        topics.len(),
        num_events
    );

    for (ind, topic) in topics.iter().enumerate() {
        let event = events_for_component[ind].clone();
        assert_eq!(&event.topic, topic);
    }
}

#[when(expr = "the indexer {word} tracks the address {word}")]
async fn track_addresss_in_indexer(world: &mut TariWorld, indexer_name: String, output_ref: String) {
    let indexer = world.indexers.get(&indexer_name).unwrap();
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);
    indexer.add_address(world, output_ref).await;
}

#[then(expr = "the indexer {word} returns version {int} for substate {word}")]
async fn assert_indexer_substate_version(
    world: &mut TariWorld,
    indexer_name: String,
    version: u32,
    output_ref: String,
) {
    let indexer = world.indexers.get(&indexer_name).unwrap();
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);
    let substate = indexer.get_substate(world, output_ref, version).await;
    eprintln!(
        "indexer.get_substate result: {}",
        serde_json::to_string_pretty(&substate).unwrap()
    );
    assert_eq!(substate.version, version);
}

#[then(expr = "the indexer {word} returns {int} non fungibles for resource {word}")]
async fn assert_indexer_non_fungible_list(
    world: &mut TariWorld,
    indexer_name: String,
    count: usize,
    output_ref: String,
) {
    let indexer = world.indexers.get(&indexer_name).unwrap();
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);
    let nfts = indexer.get_non_fungibles(world, output_ref, 0, count as u64).await;
    eprintln!("indexer.get_non_fungibles result: {:?}", nfts);
    assert_eq!(
        nfts.len(),
        count,
        "Unexpected number of NFTs returned. Expected: {}, Actual: {}",
        count,
        nfts.len()
    );
}
