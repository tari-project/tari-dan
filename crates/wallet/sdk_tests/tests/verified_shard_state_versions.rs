//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A verifying wallet reads the state version of every shard from the latest checkpoint of each shard group, which it
//! asks its indexer for in one request. It verifies a group's checkpoint only when it ends another epoch than the one
//! verified before, and rejects a list of checkpoints whose shard groups do not hold every shard once.

mod support;

use std::collections::HashMap;

use tari_ootle_common_types::{Epoch, ShardGroup, StateVersion, shard::Shard};
use tari_ootle_light_client::test_fixtures::{CheckpointSpec, ValidatorSetFixture, checkpoint_entry, high};
use tari_ootle_wallet_sdk::{IndexerTrust, apis::validator_set::ValidatorSetApiError, models::CheckpointStateVersion};

use crate::support::{ProvingNetwork, TestWithNetwork};

const LOW_SHARD: Shard = Shard::from_u32(1);
const HIGH_SHARD: Shard = Shard::from_u32(3);

/// Validator sets of epochs 1 to 4.
fn sets() -> Vec<ValidatorSetFixture> {
    (1..=4).map(ValidatorSetFixture::new).collect()
}

/// The checkpoint of [`high`] ending `sets[index]`'s epoch with [`HIGH_SHARD`] at `state_version`, signed by `signers`'
/// committee of that group.
fn high_checkpoint(
    sets: &[ValidatorSetFixture],
    index: usize,
    signers: &ValidatorSetFixture,
    state_version: u64,
) -> tari_indexer_client::types::EpochCheckpointEntry {
    checkpoint_entry(CheckpointSpec {
        shard_group: high(),
        signers: &signers.high,
        next_epoch_committee: Some(sets[index + 1].committee(high())),
        shard_state_versions: vec![(HIGH_SHARD, state_version)],
        ..CheckpointSpec::handover(&sets[index], &sets[index + 1])
    })
}

/// Makes the latest checkpoints end `sets[index]`'s epoch, with [`LOW_SHARD`] at `state_version`.
fn end_low_epoch(network: &ProvingNetwork, sets: &[ValidatorSetFixture], index: usize, state_version: u64) {
    network.replace_checkpoint(
        index,
        checkpoint_entry(CheckpointSpec {
            shard_state_versions: vec![(LOW_SHARD, state_version)],
            ..CheckpointSpec::handover(&sets[index], &sets[index + 1])
        }),
    );
    network.set_latest(index);
}

/// A verifying wallet anchored on the validator set of epoch 2, and an indexer whose latest checkpoints of both shard
/// groups end epoch 2.
async fn anchored_wallet(sets: &[ValidatorSetFixture]) -> (ProvingNetwork, TestWithNetwork<ProvingNetwork>) {
    let network = ProvingNetwork::new(sets, 0);
    let test = TestWithNetwork::with_network_trusting(network.clone(), IndexerTrust::Verify);
    test.sdk().validator_set_api().sync().await.unwrap();
    end_low_epoch(&network, sets, 1, 10);
    network.serve_latest_checkpoint(high_checkpoint(sets, 1, &sets[1], 7));
    (network, test)
}

async fn verified_versions(test: &TestWithNetwork<ProvingNetwork>) -> HashMap<Shard, CheckpointStateVersion> {
    test.sdk()
        .validator_set_api()
        .verified_shard_state_versions()
        .await
        .unwrap()
}

async fn verification_error(test: &TestWithNetwork<ProvingNetwork>) -> ValidatorSetApiError {
    test.sdk()
        .validator_set_api()
        .verified_shard_state_versions()
        .await
        .expect_err("the shard state versions verified")
}

fn version(epoch: u64, state_version: u64) -> CheckpointStateVersion {
    CheckpointStateVersion {
        epoch: Epoch(epoch),
        state_version: StateVersion::from(state_version),
    }
}

#[tokio::test]
async fn each_round_asks_for_every_shard_group_in_one_request() {
    let sets = sets();
    let (network, test) = anchored_wallet(&sets).await;

    for round in 1..=3 {
        let versions = verified_versions(&test).await;
        assert_eq!(versions[&LOW_SHARD], version(2, 10));
        assert_eq!(versions[&HIGH_SHARD], version(2, 7));
        assert_eq!(network.shard_group_checkpoint_requests(), round);
    }
    assert_eq!(
        network.committee_requests(),
        1,
        "only the high group's committee is not the walked one"
    );
}

/// Within an epoch the wallet keeps what it verified, so a checkpoint that would not verify goes unread.
#[tokio::test]
async fn a_shard_group_checkpoint_of_the_epoch_verified_before_is_not_verified_again() {
    let sets = sets();
    let (network, test) = anchored_wallet(&sets).await;
    verified_versions(&test).await;

    network.serve_latest_checkpoint(high_checkpoint(&sets, 1, &sets[0], 99));
    let versions = verified_versions(&test).await;
    assert_eq!(versions[&HIGH_SHARD], version(2, 7));
}

#[tokio::test]
async fn only_the_shard_groups_whose_latest_checkpoint_ends_a_later_epoch_are_verified_again() {
    let sets = sets();
    let (network, test) = anchored_wallet(&sets).await;
    verified_versions(&test).await;

    end_low_epoch(&network, &sets, 2, 20);
    network.serve_latest_checkpoint(high_checkpoint(&sets, 1, &sets[0], 99));
    let versions = verified_versions(&test).await;
    assert_eq!(versions[&LOW_SHARD], version(3, 20));
    assert_eq!(
        versions[&HIGH_SHARD],
        version(2, 7),
        "the high group's epoch is unchanged"
    );

    let committee_requests = network.committee_requests();
    network.serve_latest_checkpoint(high_checkpoint(&sets, 2, &sets[2], 15));
    let versions = verified_versions(&test).await;
    assert_eq!(versions[&HIGH_SHARD], version(3, 15));
    assert_eq!(network.committee_requests(), committee_requests + 1);
    assert_eq!(network.shard_group_checkpoint_requests(), 3);
}

#[tokio::test]
async fn a_later_checkpoint_that_does_not_verify_is_rejected() {
    let sets = sets();
    let (network, test) = anchored_wallet(&sets).await;
    verified_versions(&test).await;

    end_low_epoch(&network, &sets, 2, 20);
    network.serve_latest_checkpoint(high_checkpoint(&sets, 2, &sets[0], 99));
    assert!(
        matches!(verification_error(&test).await, ValidatorSetApiError::Unverified(_)),
        "a checkpoint signed by another committee"
    );
}

#[tokio::test]
async fn a_list_that_leaves_a_shard_without_a_shard_group_is_rejected() {
    let sets = sets();
    let (network, test) = anchored_wallet(&sets).await;
    network.list_shard_group_checkpoints(vec![checkpoint_entry(CheckpointSpec::handover(&sets[1], &sets[2]))]);
    assert!(matches!(
        verification_error(&test).await,
        ValidatorSetApiError::ShardWithoutCheckpoint { shard } if shard == HIGH_SHARD
    ));
}

#[tokio::test]
async fn a_list_that_holds_a_shard_in_two_shard_groups_is_rejected() {
    let sets = sets();
    let (network, test) = anchored_wallet(&sets).await;
    network.list_shard_group_checkpoints(vec![
        checkpoint_entry(CheckpointSpec::handover(&sets[1], &sets[2])),
        checkpoint_entry(CheckpointSpec {
            shard_group: ShardGroup::new(2u32, 4u32),
            ..CheckpointSpec::handover(&sets[1], &sets[2])
        }),
    ]);
    assert!(matches!(
        verification_error(&test).await,
        ValidatorSetApiError::ShardInTwoCheckpoints { shard } if shard == Shard::from_u32(2)
    ));
}

#[tokio::test]
async fn a_list_with_a_shard_group_outside_the_network_is_rejected() {
    let sets = sets();
    let (network, test) = anchored_wallet(&sets).await;
    let outside = ShardGroup::new(3u32, 5u32);
    network.list_shard_group_checkpoints(vec![
        checkpoint_entry(CheckpointSpec::handover(&sets[1], &sets[2])),
        checkpoint_entry(CheckpointSpec {
            shard_group: outside,
            ..CheckpointSpec::handover(&sets[1], &sets[2])
        }),
    ]);
    assert!(matches!(
        verification_error(&test).await,
        ValidatorSetApiError::CheckpointOfShardsOutsideNetwork { shard_group } if shard_group == outside
    ));
}

#[tokio::test]
async fn an_empty_list_has_no_shard_state_versions() {
    let sets = sets();
    let (network, test) = anchored_wallet(&sets).await;
    network.list_shard_group_checkpoints(vec![]);
    assert!(verified_versions(&test).await.is_empty());
}
