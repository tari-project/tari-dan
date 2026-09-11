//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

use cucumber::{gherkin::Step, given, then, when};
use integration_tests::{
    TariWorld,
    cucumber_log,
    indexer::{IndexerProcess, spawn_indexer},
};
use libp2p::Multiaddr;
use tari_ootle_common_types::{Epoch, StateVersion, displayable::Displayable, optional::Optional, shard::Shard};

#[when(expr = "indexer {word} connects to all other validators")]
async fn given_validator_connects_to_other_vns(world: &mut TariWorld, name: String) {
    let indexer = world.get_indexer(&name);
    let details = world
        .all_running_validators_iter()
        .filter(|vn| vn.name != name)
        .map(|vn| {
            (
                vn.public_key,
                Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", vn.p2p_port)).unwrap(),
            )
        });

    for (pk, addr) in details {
        indexer.add_peer(pk, vec![addr.clone()]).await;
    }
}

/// The number of shard groups follows the registered validator count divided by the committee size,
/// so this is how a scenario states the shape of the network it has built - and waits for a
/// registration to take effect at an epoch boundary.
///
/// Every shard group is also required to have a committee. Validators are placed in the shard space
/// by a base-layer shard key that the scenario does not choose, so a small network can be dealt a
/// split that leaves one half with no validators at all. Nothing can answer for that half, and
/// every later step that touches it fails somewhere far from the cause, so it is named here.
#[then(expr = "the network has {int} shard group(s) according to indexer {word}")]
async fn network_has_shard_groups(world: &mut TariWorld, step: &Step, num_shard_groups: usize, name: String) {
    cucumber_log!("=== Step:{}", step.value);
    let client = world.get_indexer(&name).get_indexer_client();
    let mut remaining = 20;
    loop {
        let state = client
            .get_network_sync_state()
            .await
            .expect("Failed to get network sync state");
        let shard_groups = &state.network_desc.shard_groups;
        let empty = shard_groups
            .iter()
            .filter(|(_, num_members)| *num_members == 0)
            .map(|(shard_group, _)| shard_group.to_string())
            .collect::<Vec<_>>();
        if shard_groups.len() == num_shard_groups && empty.is_empty() {
            return;
        }

        if remaining == 0 {
            if shard_groups.len() == num_shard_groups {
                panic!(
                    "Indexer {} sees the expected {} shard group(s) at epoch {}, but no validator is assigned to {}. \
                     Every validator shard key landed in the other part of the shard space, so nothing can answer for \
                     this one. Shard groups: {:?}",
                    name,
                    num_shard_groups,
                    state.network_desc.epoch,
                    empty.join(", "),
                    shard_groups
                );
            }
            panic!(
                "Indexer {} sees {} shard group(s) at epoch {}, expected {}: {:?}",
                name,
                shard_groups.len(),
                state.network_desc.epoch,
                num_shard_groups,
                shard_groups
            );
        }
        remaining -= 1;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

#[then(expr = "indexer {word} has scanned to at least height {int}")]
pub async fn indexer_has_scanned_to_at_least_height(
    world: &mut TariWorld,
    step: &Step,
    name: String,
    block_height: u64,
) {
    cucumber_log!("=== Step:{}", step.value);
    let indexer = world.get_indexer(&name);
    let client = indexer.get_indexer_client();
    let mut last_block_height = 0;
    let mut remaining = 10;
    loop {
        let stats = client.get_epoch_manager_stats().await.expect("Failed to get stats");
        if stats.current_block_height >= block_height {
            return;
        }

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

#[given(expr = "an indexer {word} connected to a base node")]
async fn start_indexer_connected_to_a_base_node(world: &mut TariWorld, indexer_name: String) {
    let bn_name = world
        .base_nodes
        .keys()
        .next()
        .cloned()
        .expect("no base nodes have been started");
    spawn_indexer(world, indexer_name, bn_name).await;
}

#[then(expr = "{word} indexer GraphQL request works")]
async fn works_indexer_graphql(world: &mut TariWorld, indexer_name: String) {
    let indexer = world.indexers.get(&indexer_name).unwrap();
    let mut graphql_client = indexer.get_graphql_indexer_client().await;
    let query = r#"{ getEvents { substateId, templateAddress, txHash, topic, payload } }"#.to_string();
    let res = graphql_client
        .send_request::<HashMap<String, Vec<tari_indexer::graphql::model::events::Event>>>(&query, None, None)
        .await
        .expect("Failed to obtain getEventsForTransaction query result");
    res.get("getEvents").unwrap();
}

#[when(expr = "indexer {word} scans the network events for account {word} with topics {word}")]
async fn indexer_scans_network_events(
    world: &mut TariWorld,
    indexer_name: String,
    account_name: String,
    topics_str: String,
) {
    let indexer: &mut IndexerProcess = world.indexers.get_mut(&indexer_name).unwrap_or_else(|| {
        panic!("Indexer {} not found", indexer_name);
    });
    let account = world.wallet_accounts.get(&account_name).unwrap_or_else(|| {
        panic!("No wallet account found with name {}", account_name);
    });
    let account_addr = account.component_address().to_string();
    let expected_topics = topics_str.split(',').map(|s| s.to_string()).collect::<Vec<_>>();

    let mut graphql_client = indexer.get_graphql_indexer_client().await;
    let query = format!(
        r#"{{ getEvents(substateId: "{}") {{ substateId, templateAddress, txHash, topic, payload }} }}"#,
        account_addr
    );

    let mut remaining_attempts = 10;
    loop {
        let res = graphql_client
            .send_request::<HashMap<String, Vec<tari_indexer::graphql::model::events::Event>>>(&query, None, None)
            .await
            .expect("Failed to obtain getEvents query result");

        let events = res.get("getEvents").unwrap();
        let topics_for_component = events.iter().map(|e| e.topic.as_str()).collect::<HashSet<_>>();

        let is_all_topics_found = expected_topics
            .iter()
            .all(|t| topics_for_component.contains(t.as_str()));

        if is_all_topics_found {
            return;
        }

        remaining_attempts -= 1;
        if remaining_attempts == 0 {
            panic!(
                "Timed out waiting for events. Events emitted for {} were {}. Expected topics: {:?} (ALL events: {:?})",
                account_addr,
                topics_for_component.display(),
                expected_topics,
                events
            );
        }

        cucumber_log!(
            "Waiting for events for {} ({} attempts remaining, found: {})",
            account_addr,
            remaining_attempts,
            topics_for_component.display()
        );
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[when(expr = "indexer {word} scans the network for events of resource {word}")]
async fn indexer_scans_network_events_for_resource(world: &mut TariWorld, indexer_name: String, resource_path: String) {
    let indexer: &mut IndexerProcess = world.indexers.get_mut(&indexer_name).unwrap();

    // extract the resource address from the outputs
    let (input_group, index) = resource_path.split_once('/').unwrap_or_else(|| {
        panic!(
            "Resource name must be in the format '{{group}}/resources/{{index}}', got {}",
            resource_path
        )
    });
    let resource_address = world
        .outputs
        .get(input_group)
        .unwrap_or_else(|| panic!("No outputs found with name {}", input_group))
        .iter()
        .find(|(i, _)| **i == index)
        .map(|(_, data)| data.clone())
        .unwrap_or_else(|| panic!("No resource with index {}", index))
        .substate_id()
        .as_resource_address()
        .unwrap_or_else(|| panic!("The output is not a resource {}", index));

    let mut graphql_client = indexer.get_graphql_indexer_client().await;
    let query = format!(
        r#"{{ getEvents(substateId:"{}", offset:0, limit:10) {{ substateId, templateAddress, txHash, topic, payload }} }}"#,
        resource_address
    );
    let res = graphql_client
        .send_request::<HashMap<String, Vec<tari_indexer::graphql::model::events::Event>>>(&query, None, None)
        .await
        .expect("Failed to obtain getEvents query result");

    let events = res.get("getEvents").unwrap();

    // TODO: assert the results
    cucumber_log!("{:?}", events);
}

#[then(expr = "the indexer {word} returns version {int} for substate {word}")]
async fn assert_indexer_substate_version(
    world: &mut TariWorld,
    indexer_name: String,
    version: u64,
    output_ref: String,
) {
    let indexer = world.indexers.get(&indexer_name).unwrap();
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);

    let mut remaining_attempts = 30usize;
    loop {
        match indexer.get_substate(world, output_ref.clone(), version).await {
            Ok(substate) => {
                cucumber_log!(
                    "indexer.get_substate result: {}",
                    serde_json::to_string_pretty(&substate).unwrap()
                );
                assert_eq!(substate.version, version);
                return;
            },
            Err(e) => {
                if remaining_attempts == 0 {
                    panic!(
                        "Indexer {} did not return version {} for substate {} in time. Last error: {}",
                        indexer_name, version, output_ref, e
                    );
                }
                remaining_attempts -= 1;
                cucumber_log!(
                    "Waiting for indexer {} to sync substate {} (version {}), {} attempts remaining. Error: {}",
                    indexer_name,
                    output_ref,
                    version,
                    remaining_attempts,
                    e
                );
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            },
        }
    }
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
    cucumber_log!("indexer.get_non_fungibles result: {:?}", nfts);
    assert_eq!(
        nfts.len(),
        count,
        "Unexpected number of NFTs returned. Expected: {}, Actual: {}",
        count,
        nfts.len()
    );
}

#[then(expr = "the indexer {word} has at least {int} template(s) in the catalogue")]
async fn assert_indexer_catalogue_count(world: &mut TariWorld, indexer_name: String, min_count: usize) {
    let indexer = world.get_indexer(&indexer_name);
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);

    let mut remaining = 30;
    loop {
        let resp = indexer.list_template_catalogue(None, Some(100), None).await;
        if resp.entries.len() >= min_count {
            return;
        }
        if remaining == 0 {
            panic!(
                "Indexer {} catalogue has {} entries, expected at least {}",
                indexer_name,
                resp.entries.len(),
                min_count
            );
        }
        remaining -= 1;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[then(expr = "the indexer {word} catalogue contains template {word}")]
async fn assert_indexer_catalogue_contains_template(
    world: &mut TariWorld,
    indexer_name: String,
    template_name: String,
) {
    let template = world
        .templates
        .get(&template_name)
        .unwrap_or_else(|| panic!("Template {} not registered in world", template_name));
    let template_address = template.address;

    let indexer = world.get_indexer(&indexer_name);
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);

    let mut remaining = 30;
    loop {
        let client = indexer.get_indexer_client();
        match client.get_template_catalogue_entry(template_address).await {
            Ok(entry) => {
                assert_eq!(
                    entry.template_address, template_address,
                    "Template address mismatch in catalogue entry"
                );
                assert!(
                    !entry.template_name.is_empty(),
                    "template_name should not be empty for {} (address: {})",
                    template_name,
                    template_address
                );
                return;
            },
            Err(_) => {
                if remaining == 0 {
                    panic!(
                        "Indexer {} catalogue does not contain template {} (address: {})",
                        indexer_name, template_name, template_address
                    );
                }
                remaining -= 1;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            },
        }
    }
}

#[then(expr = "the indexer {word} catalogue name filter {word} returns {int} result(s)")]
async fn assert_indexer_catalogue_name_filter(
    world: &mut TariWorld,
    indexer_name: String,
    name_filter: String,
    expected_count: usize,
) {
    let indexer = world.get_indexer(&indexer_name);
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);
    let resp = indexer
        .list_template_catalogue(Some(name_filter.clone()), Some(100), None)
        .await;
    assert_eq!(
        resp.entries.len(),
        expected_count,
        "Catalogue name filter '{}' returned {} entries, expected {}",
        name_filter,
        resp.entries.len(),
        expected_count
    );
}

#[then(expr = "the indexer {word} catalogue with limit {int} returns {int} entries")]
async fn assert_indexer_catalogue_page(world: &mut TariWorld, indexer_name: String, limit: u64, expected_count: usize) {
    let indexer = world.get_indexer(&indexer_name);
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);
    let resp = indexer.list_template_catalogue(None, Some(limit), None).await;
    assert_eq!(
        resp.entries.len(),
        expected_count,
        "Catalogue with limit={} returned {} entries, expected {}",
        limit,
        resp.entries.len(),
        expected_count
    );
}

#[then(expr = "the indexer {word} catalogue with limit {int} returns at least {int} entries")]
async fn assert_indexer_catalogue_page_min(world: &mut TariWorld, indexer_name: String, limit: u64, min_count: usize) {
    let indexer = world.get_indexer(&indexer_name);
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);
    let resp = indexer.list_template_catalogue(None, Some(limit), None).await;
    assert!(
        resp.entries.len() >= min_count,
        "Catalogue with limit={} returned {} entries, expected at least {}",
        limit,
        resp.entries.len(),
        min_count
    );
}

#[then(expr = "the indexer {word} catalogue entry for address {string} is not found")]
async fn assert_catalogue_entry_not_found(world: &mut TariWorld, indexer_name: String, address_str: String) {
    use tari_engine_types::published_template::PublishedTemplateAddress;
    let address = PublishedTemplateAddress::from_str(&address_str)
        .unwrap_or_else(|_| panic!("Invalid template address: {}", address_str))
        .as_template_address();
    let indexer = world.get_indexer(&indexer_name);
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);
    let client = indexer.get_indexer_client();
    let item = client.get_template_catalogue_entry(address).await.optional().unwrap();
    assert!(
        item.is_none(),
        "Expected not found for address {} but got a result",
        address_str
    );
}

#[then(expr = "I wait for the indexer {word} to sync with the network")]
async fn i_wait_for_the_indexer_to_sync_with_the_network(world: &mut TariWorld, indexer_name: String) {
    // A validator reports state versions for its own shard group only, so the target is the union
    // over every running validator: on a network of more than one committee, a single validator
    // describes half the shard space and says nothing about the half another committee holds.
    let mut epoch = None;
    let mut state_versions: HashMap<Shard, StateVersion> = HashMap::new();
    for vn in world
        .validator_nodes
        .values()
        .chain(world.vn_seeds.values())
        .filter(|vn| !vn.shutdown.is_triggered())
    {
        let consensus_stats = vn
            .get_client()
            .get_consensus_status()
            .await
            .expect("Failed to get epoch stats from VN");
        if consensus_stats.state != "Running" {
            continue;
        }
        epoch = Some(consensus_stats.epoch);
        for (shard, version) in consensus_stats.state_versions.unwrap_or_default() {
            state_versions
                .entry(shard)
                .and_modify(|v| *v = (*v).max(version))
                .or_insert(version);
        }
    }

    let epoch = epoch.expect(
        "No running validator nodes found. An indexer must be connected to a running validator node to sync with the \
         network",
    );
    let prev_epoch = epoch.checked_sub(Epoch(1)).expect("Epoch is zero");
    assert!(
        !state_versions.is_empty(),
        "No state versions found in consensus stats for any running validator"
    );

    let indexer = world.get_indexer(&indexer_name);
    assert!(!indexer.handle.is_finished(), "Indexer {} is not running", indexer_name);
    let client = indexer.get_indexer_client();
    let mut remaining_attempts = 120;
    loop {
        if remaining_attempts == 0 {
            panic!(
                "Indexer {} did not sync with the network in time. Current epoch: {}",
                indexer_name, prev_epoch
            );
        }

        remaining_attempts -= 1;
        let state = client.get_network_sync_state().await.unwrap();
        if let Some(ref progress) = state.sync_progress {
            // The indexer is synced once it has scanned every shard up to the version the network has
            // committed. Readiness is a per-shard version comparison, not an epoch comparison: a shard
            // whose last change was in an earlier epoch has no transition to advance its scanned epoch
            // into prev_epoch, so requiring `scanned_epoch >= prev_epoch` stalls forever on idle shards.
            // Shards with no committed state (version 0) require nothing to be synced.
            let indexer_version_for = |shard: Shard| {
                progress
                    .last_state_versions
                    .iter()
                    .find(|(s, _)| *s == shard)
                    .map(|(_, (v, _))| *v)
            };
            if let Some((shard, network_version)) = state_versions
                .iter()
                .filter(|(_, sv)| sv.as_u64() > 0)
                .find(|(s, sv)| indexer_version_for(**s).is_none_or(|v| v < **sv))
            {
                let scanned_version = indexer_version_for(*shard);
                integration_tests::cucumber_log!(
                    "Waiting for indexer {} to sync. Current epoch: {}, shard: {}, network version: {}, indexer \
                     version: {}",
                    indexer_name,
                    prev_epoch,
                    shard,
                    network_version,
                    scanned_version.display()
                );
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }

            break;
        } else {
            integration_tests::cucumber_log!(
                "Waiting for indexer {} to sync. Current epoch: {}, no sync progress yet",
                indexer_name,
                prev_epoch
            );
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
}
